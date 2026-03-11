use std::ffi::c_void;
use std::sync::Arc;

use libffi::middle::{arg, Arg, Cif, CodePtr};
use napi::bindgen_prelude::*;
use napi::{JsObject, JsUnknown, NapiRaw, NapiValue, Result};

use crate::cif::ffi_type_for;
use crate::ffi_type::FfiTypeDesc;
use crate::library::LibraryHandle;
use crate::marshal;

/// C layout of RustBuffer { capacity: u64, len: u64, data: *mut u8 }
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RustBufferC {
    pub capacity: u64,
    pub len: u64,
    pub data: *mut u8,
}

/// C layout of ForeignBytes { len: i32, data: *const u8 }
#[repr(C)]
struct ForeignBytesC {
    len: i32,
    data: *const u8,
}

/// C layout of RustCallStatus, matching the test_lib definition.
#[repr(C)]
struct RustCallStatusC {
    code: i8,
    // RustBuffer: capacity, len, data
    error_buf_capacity: u64,
    error_buf_len: u64,
    error_buf_data: *mut u8,
}

type RustBufferFromBytesFn = unsafe extern "C" fn(ForeignBytesC, *mut RustCallStatusC) -> RustBufferC;
type RustBufferFreeFn = unsafe extern "C" fn(RustBufferC, *mut RustCallStatusC);

impl Default for RustCallStatusC {
    fn default() -> Self {
        Self {
            code: 0,
            error_buf_capacity: 0,
            error_buf_len: 0,
            error_buf_data: std::ptr::null_mut(),
        }
    }
}

pub fn register(env: Env, handle: &LibraryHandle, definitions: JsObject) -> Result<JsObject> {
    let functions: JsObject = definitions.get_named_property("functions")?;
    let mut result = env.create_object()?;

    let names = functions.get_property_names()?;
    let len = names.get_array_length()?;

    for i in 0..len {
        let name: String = names.get_element::<napi::JsString>(i)?.into_utf8()?.as_str()?.to_owned();
        let func_def: JsObject = functions.get_named_property(&name)?;

        // Parse argument types
        let args_arr: JsObject = func_def.get_named_property("args")?;
        let args_len = args_arr.get_array_length()?;
        let mut arg_types = Vec::with_capacity(args_len as usize);
        for j in 0..args_len {
            let arg_obj: JsObject = args_arr.get_element(j)?;
            arg_types.push(FfiTypeDesc::from_js_object(&env, &arg_obj)?);
        }

        // Parse return type
        let ret_obj: JsObject = func_def.get_named_property("ret")?;
        let ret_type = FfiTypeDesc::from_js_object(&env, &ret_obj)?;

        // Check hasRustCallStatus
        let has_rust_call_status: bool = func_def.get_named_property("hasRustCallStatus")?;

        // Look up symbol
        let symbol_ptr = handle.lookup_symbol(&name)?;

        // Build CIF: declared args + optional RustCallStatus pointer
        let mut cif_arg_types: Vec<libffi::middle::Type> =
            arg_types.iter().map(ffi_type_for).collect();
        if has_rust_call_status {
            cif_arg_types.push(libffi::middle::Type::pointer());
        }
        let cif_ret_type = ffi_type_for(&ret_type);
        let cif = Cif::new(cif_arg_types, cif_ret_type);

        // Wrap in Arc so the closure can own it
        let cif = Arc::new(cif);
        let arg_types = Arc::new(arg_types);
        let ret_type_clone = ret_type.clone();

        // Capture rustbuffer function pointers for RustBuffer arg/ret handling
        let rb_from_bytes_ptr = handle.rustbuffer_from_bytes;
        let rb_free_ptr = handle.rustbuffer_free;

        let js_func = env.create_function_from_closure(&name, move |ctx| {
            call_ffi_function(
                ctx.env,
                &ctx,
                &cif,
                symbol_ptr,
                &arg_types,
                &ret_type_clone,
                has_rust_call_status,
                rb_from_bytes_ptr,
                rb_free_ptr,
            )
        })?;

        result.set_named_property(&name, js_func)?;
    }

    Ok(result)
}

fn call_ffi_function(
    env: &Env,
    ctx: &napi::CallContext<'_>,
    cif: &Cif,
    symbol_ptr: *const c_void,
    arg_types: &[FfiTypeDesc],
    ret_type: &FfiTypeDesc,
    has_rust_call_status: bool,
    rb_from_bytes_ptr: *const c_void,
    rb_free_ptr: *const c_void,
) -> Result<JsUnknown> {
    let declared_arg_count = arg_types.len();

    // Marshal JS arguments to boxed Rust values
    let mut boxed_args: Vec<Box<dyn std::any::Any>> = Vec::with_capacity(declared_arg_count);
    for (i, desc) in arg_types.iter().enumerate() {
        let js_val: JsUnknown = ctx.get(i)?;
        match desc {
            FfiTypeDesc::RustBuffer => {
                // Convert Uint8Array -> RustBufferC via rustbuffer_from_bytes
                let rust_buffer = js_uint8array_to_rust_buffer(env, js_val, rb_from_bytes_ptr)?;
                boxed_args.push(Box::new(rust_buffer));
            }
            _ => {
                boxed_args.push(marshal::js_to_boxed(env, desc, js_val)?);
            }
        }
    }

    // Build libffi Arg references
    let mut ffi_args: Vec<Arg> = Vec::with_capacity(declared_arg_count + 1);
    for (i, desc) in arg_types.iter().enumerate() {
        match desc {
            FfiTypeDesc::RustBuffer => {
                ffi_args.push(arg(boxed_args[i].downcast_ref::<RustBufferC>().unwrap()));
            }
            _ => {
                ffi_args.push(marshal::boxed_to_arg(desc, boxed_args[i].as_ref()));
            }
        }
    }

    // Handle RustCallStatus
    let mut rust_call_status = RustCallStatusC::default();
    let mut status_ptr: *mut RustCallStatusC = std::ptr::null_mut();
    let mut status_js_obj: Option<JsObject> = None;

    if has_rust_call_status {
        // The last JS argument is the status object { code: number }
        let status_idx = declared_arg_count;
        let js_status: JsObject = ctx.get(status_idx)?;
        let code_val: i32 = js_status.get_named_property("code")?;
        rust_call_status.code = code_val as i8;
        status_js_obj = Some(js_status);

        // Pass pointer to rust_call_status as the last C arg
        status_ptr = &mut rust_call_status as *mut RustCallStatusC;
        ffi_args.push(arg(&status_ptr));
    }

    // Call the function
    let code_ptr = CodePtr::from_ptr(symbol_ptr as *mut c_void);
    let ret_val: Box<dyn std::any::Any> = call_with_ret_type(cif, code_ptr, &ffi_args, ret_type)?;

    // Write back RustCallStatus
    if has_rust_call_status {
        if let Some(mut js_status) = status_js_obj {
            js_status.set_named_property("code", env.create_int32(rust_call_status.code as i32)?)?;
        }
    }

    // Marshal return value to JS
    match ret_type {
        FfiTypeDesc::RustBuffer => {
            let rb = ret_val.downcast_ref::<RustBufferC>().unwrap();
            rust_buffer_to_js_uint8array(env, *rb, rb_free_ptr)
        }
        _ => marshal::ret_to_js(env, ret_type, ret_val.as_ref()),
    }
}

/// Call the CIF with the correct return type based on FfiTypeDesc.
fn call_with_ret_type(
    cif: &Cif,
    code_ptr: CodePtr,
    args: &[Arg],
    ret_type: &FfiTypeDesc,
) -> Result<Box<dyn std::any::Any>> {
    unsafe {
        match ret_type {
            FfiTypeDesc::Void => {
                cif.call::<()>(code_ptr, args);
                Ok(Box::new(()))
            }
            FfiTypeDesc::UInt8 => {
                let r: u8 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::Int8 => {
                let r: i8 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::UInt16 => {
                let r: u16 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::Int16 => {
                let r: i16 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::UInt32 => {
                let r: u32 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::Int32 => {
                let r: i32 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => {
                let r: u64 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::Int64 => {
                let r: i64 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::Float32 => {
                let r: f32 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::Float64 => {
                let r: f64 = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            FfiTypeDesc::RustBuffer => {
                let r: RustBufferC = cif.call(code_ptr, args);
                Ok(Box::new(r))
            }
            _ => Err(napi::Error::from_reason(format!(
                "Unsupported return type: {:?}",
                ret_type
            ))),
        }
    }
}

/// Convert a JS Uint8Array to a RustBufferC by calling rustbuffer_from_bytes.
fn js_uint8array_to_rust_buffer(
    env: &Env,
    js_val: JsUnknown,
    rb_from_bytes_ptr: *const c_void,
) -> Result<RustBufferC> {
    // Extract the typed array data using napi sys
    let raw_env = env.raw();
    let raw_val = unsafe { js_val.raw() };

    let mut length: usize = 0;
    let mut data: *mut c_void = std::ptr::null_mut();
    let mut arraybuffer = std::ptr::null_mut();
    let mut byte_offset: usize = 0;
    let mut typedarray_type: i32 = 0;

    let status_code = unsafe {
        napi::sys::napi_get_typedarray_info(
            raw_env,
            raw_val,
            &mut typedarray_type,
            &mut length,
            &mut data,
            &mut arraybuffer,
            &mut byte_offset,
        )
    };
    if status_code != napi::sys::Status::napi_ok {
        return Err(napi::Error::from_reason(
            "Expected a Uint8Array argument for RustBuffer".to_string(),
        ));
    }

    let data_ptr = data as *const u8;
    let foreign = ForeignBytesC {
        len: length as i32,
        data: if length > 0 { data_ptr } else { std::ptr::null() },
    };

    let mut call_status = RustCallStatusC::default();
    let from_bytes: RustBufferFromBytesFn = unsafe { std::mem::transmute(rb_from_bytes_ptr) };
    let rb = unsafe { from_bytes(foreign, &mut call_status as *mut RustCallStatusC) };

    if call_status.code != 0 {
        return Err(napi::Error::from_reason(
            "rustbuffer_from_bytes failed".to_string(),
        ));
    }

    Ok(rb)
}

/// Convert a RustBufferC to a JS Uint8Array, then free the buffer.
fn rust_buffer_to_js_uint8array(
    env: &Env,
    rb: RustBufferC,
    rb_free_ptr: *const c_void,
) -> Result<JsUnknown> {
    let len = rb.len as usize;
    let raw_env = env.raw();

    // Create an ArrayBuffer and copy data into it
    let mut arraybuffer_data: *mut c_void = std::ptr::null_mut();
    let mut arraybuffer = std::ptr::null_mut();

    let status_code = unsafe {
        napi::sys::napi_create_arraybuffer(raw_env, len, &mut arraybuffer_data, &mut arraybuffer)
    };
    if status_code != napi::sys::Status::napi_ok {
        return Err(napi::Error::from_reason(
            "Failed to create ArrayBuffer".to_string(),
        ));
    }

    if len > 0 && !rb.data.is_null() {
        unsafe {
            std::ptr::copy_nonoverlapping(rb.data, arraybuffer_data as *mut u8, len);
        }
    }

    // Create a Uint8Array view over the ArrayBuffer
    let mut typedarray = std::ptr::null_mut();
    let status_code = unsafe {
        napi::sys::napi_create_typedarray(
            raw_env,
            1, // napi_uint8_array (Int8=0, Uint8=1)
            len,
            arraybuffer,
            0,
            &mut typedarray,
        )
    };
    if status_code != napi::sys::Status::napi_ok {
        return Err(napi::Error::from_reason(
            "Failed to create Uint8Array".to_string(),
        ));
    }

    // Free the RustBuffer (if non-null)
    if !rb.data.is_null() && rb.capacity > 0 {
        let free_fn: RustBufferFreeFn = unsafe { std::mem::transmute(rb_free_ptr) };
        let mut call_status = RustCallStatusC::default();
        unsafe { free_fn(rb, &mut call_status as *mut RustCallStatusC) };
    }

    Ok(unsafe { JsUnknown::from_raw(raw_env, typedarray)? })
}
