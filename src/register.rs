use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;

use libffi::middle::{arg, Arg, Cif, Closure, CodePtr};
use napi::bindgen_prelude::*;
use napi::{JsObject, JsUnknown, NapiRaw, NapiValue, Result};

use crate::callback::{self, raw_arg_to_js, CallbackDef, RawCallbackArg, TrampolineUserdata};
use crate::cif::ffi_type_for;
use crate::ffi_type::FfiTypeDesc;
use crate::library::LibraryHandle;
use crate::marshal;
use crate::structs;

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

type RustBufferFromBytesFn =
    unsafe extern "C" fn(ForeignBytesC, *mut RustCallStatusC) -> RustBufferC;
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
    // Parse callback definitions if present
    let callback_defs = parse_callbacks(&env, &definitions)?;
    let callback_defs = Rc::new(callback_defs);

    // Parse struct definitions if present
    let struct_defs = structs::parse_structs(&env, &definitions)?;
    let struct_defs = Rc::new(struct_defs);

    let functions: JsObject = definitions.get_named_property("functions")?;
    let mut result = env.create_object()?;

    let names = functions.get_property_names()?;
    let len = names.get_array_length()?;

    for i in 0..len {
        let name: String = names
            .get_element::<napi::JsString>(i)?
            .into_utf8()?
            .as_str()?
            .to_owned();
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

        // Wrap in Rc so the closure can own it (single-threaded napi context)
        let cif = std::rc::Rc::new(cif);
        let arg_types = Rc::new(arg_types);
        let ret_type_clone = ret_type.clone();
        let cb_defs = callback_defs.clone();
        let st_defs = struct_defs.clone();

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
                &cb_defs,
                &st_defs,
            )
        })?;

        result.set_named_property(&name, js_func)?;
    }

    Ok(result)
}

/// Parse the `callbacks` map from JS definitions into a HashMap of CallbackDefs.
fn parse_callbacks(env: &Env, definitions: &JsObject) -> Result<HashMap<String, CallbackDef>> {
    let mut map = HashMap::new();

    // callbacks is optional
    let has_callbacks: bool = definitions.has_named_property("callbacks")?;
    if !has_callbacks {
        return Ok(map);
    }

    let callbacks: JsObject = definitions.get_named_property("callbacks")?;
    let names = callbacks.get_property_names()?;
    let len = names.get_array_length()?;

    for i in 0..len {
        let name: String = names
            .get_element::<napi::JsString>(i)?
            .into_utf8()?
            .as_str()?
            .to_owned();
        let cb_def: JsObject = callbacks.get_named_property(&name)?;

        // Parse args
        let args_arr: JsObject = cb_def.get_named_property("args")?;
        let args_len = args_arr.get_array_length()?;
        let mut args = Vec::with_capacity(args_len as usize);
        for j in 0..args_len {
            let arg_obj: JsObject = args_arr.get_element(j)?;
            args.push(FfiTypeDesc::from_js_object(env, &arg_obj)?);
        }

        // Parse ret
        let ret_obj: JsObject = cb_def.get_named_property("ret")?;
        let ret = FfiTypeDesc::from_js_object(env, &ret_obj)?;

        // Parse hasRustCallStatus
        let has_rust_call_status: bool = cb_def.get_named_property("hasRustCallStatus")?;

        map.insert(
            name,
            CallbackDef {
                args,
                ret,
                has_rust_call_status,
            },
        );
    }

    Ok(map)
}

#[allow(clippy::too_many_arguments)]
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
    callback_defs: &HashMap<String, CallbackDef>,
    struct_defs: &HashMap<String, structs::StructDef>,
) -> Result<JsUnknown> {
    let declared_arg_count = arg_types.len();

    // Storage for callback closures that must live until after ffi_call.
    // We use Box<TrampolineUserdata> so the userdata has a stable address,
    // then create a Closure that borrows from it.
    // _callback_keepalive keeps both alive until this function returns.
    let mut _callback_keepalive: Vec<(Box<TrampolineUserdata>, Closure<'_>)> = Vec::new();
    // The actual function pointer values, stored separately so we can borrow them for ffi args.
    let mut callback_fn_ptrs: Vec<*const c_void> = Vec::new();

    // Storage for struct (VTable) pointers passed by reference.
    let mut struct_ptrs: Vec<*const c_void> = Vec::new();

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
            FfiTypeDesc::Reference(inner) if matches!(inner.as_ref(), FfiTypeDesc::Struct(_)) => {
                // Reference(Struct("Name")) — build a VTable struct from a JS object
                let struct_name = match inner.as_ref() {
                    FfiTypeDesc::Struct(name) => name,
                    _ => unreachable!(),
                };
                let struct_def = struct_defs.get(struct_name).ok_or_else(|| {
                    napi::Error::from_reason(format!("Unknown struct: {}", struct_name))
                })?;

                // Get the JS object for this argument
                let js_obj = unsafe { JsObject::from_raw(env.raw(), js_val.raw())? };

                // Build the C struct (VTable) from the JS object
                let struct_ptr =
                    structs::build_vtable_struct(env, struct_def, &js_obj, callback_defs)?;
                struct_ptrs.push(struct_ptr);

                // Placeholder in boxed_args
                boxed_args.push(Box::new(()));
            }
            FfiTypeDesc::Callback(cb_name) => {
                // Look up callback definition
                let cb_def = callback_defs.get(cb_name).ok_or_else(|| {
                    napi::Error::from_reason(format!("Unknown callback: {}", cb_name))
                })?;

                // Get the JS function for creating a ThreadsafeFunction
                let js_fn = unsafe { napi::JsFunction::from_raw(env.raw(), js_val.raw())? };
                let raw_fn = unsafe { js_val.raw() };

                // Create a ThreadsafeFunction for cross-thread dispatch.
                // The callback converts RawCallbackArg values to JS values on the main thread.
                let tsfn: napi::threadsafe_function::ThreadsafeFunction<
                    Vec<RawCallbackArg>,
                    napi::threadsafe_function::ErrorStrategy::Fatal,
                > = js_fn.create_threadsafe_function(
                    0,
                    |ctx: napi::threadsafe_function::ThreadSafeCallContext<Vec<RawCallbackArg>>| {
                        let mut js_args: Vec<napi::JsUnknown> = Vec::with_capacity(ctx.value.len());
                        for raw_arg in &ctx.value {
                            js_args.push(raw_arg_to_js(&ctx.env, raw_arg)?);
                        }
                        Ok(js_args)
                    },
                )?;

                // Unref the TSF so it doesn't keep the Node.js event loop alive.
                // The TSF will still work when called, but won't prevent process exit.
                let mut tsfn = tsfn;
                tsfn.unref(env)?;

                // Create userdata on the heap with a stable address
                let userdata = Box::new(TrampolineUserdata {
                    raw_env: env.raw(),
                    raw_fn,
                    arg_types: cb_def.args.clone(),
                    tsfn: Some(tsfn),
                });

                // Build the callback CIF
                let cb_cif = callback::build_callback_cif(cb_def);

                // Leak the userdata so it survives beyond this function call.
                // This is necessary because the callback may be invoked from another
                // thread after call_ffi_function returns.
                let userdata_ptr = Box::into_raw(userdata);
                let userdata_ref: &'static TrampolineUserdata = unsafe { &*userdata_ptr };

                // Create the closure with 'static lifetime since userdata is leaked.
                let closure = Closure::new(cb_cif, callback::trampoline_callback, userdata_ref);

                // Extract the function pointer value from the closure.
                let fn_ptr: *const c_void = *closure.code_ptr() as *const std::ffi::c_void;
                callback_fn_ptrs.push(fn_ptr);

                // Leak the closure too so the function pointer remains valid.
                // For cross-thread callbacks, we can't know when the callback will
                // no longer be called, so we must keep it alive indefinitely.
                std::mem::forget(closure);

                // Placeholder for boxed_args (not used for callbacks)
                boxed_args.push(Box::new(()));
            }
            _ => {
                boxed_args.push(marshal::js_to_boxed(env, desc, js_val)?);
            }
        }
    }

    // Build libffi Arg references
    let mut ffi_args: Vec<Arg> = Vec::with_capacity(declared_arg_count + 1);
    let mut cb_ptr_idx = 0;
    let mut struct_ptr_idx = 0;
    for (i, desc) in arg_types.iter().enumerate() {
        match desc {
            FfiTypeDesc::RustBuffer => {
                ffi_args.push(arg(boxed_args[i].downcast_ref::<RustBufferC>().unwrap()));
            }
            FfiTypeDesc::Callback(_) => {
                ffi_args.push(arg(&callback_fn_ptrs[cb_ptr_idx]));
                cb_ptr_idx += 1;
            }
            FfiTypeDesc::Reference(inner) if matches!(inner.as_ref(), FfiTypeDesc::Struct(_)) => {
                ffi_args.push(arg(&struct_ptrs[struct_ptr_idx]));
                struct_ptr_idx += 1;
            }
            _ => {
                ffi_args.push(marshal::boxed_to_arg(desc, boxed_args[i].as_ref()));
            }
        }
    }

    // Handle RustCallStatus
    let mut rust_call_status = RustCallStatusC::default();
    let status_ptr: *mut RustCallStatusC;
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
            js_status
                .set_named_property("code", env.create_int32(rust_call_status.code as i32)?)?;

            // If error, copy error_buf into a Uint8Array and set on js_status
            if rust_call_status.code != 0 && !rust_call_status.error_buf_data.is_null() {
                let len = rust_call_status.error_buf_len as usize;
                let raw_env = env.raw();

                let mut arraybuffer_data: *mut c_void = std::ptr::null_mut();
                let mut arraybuffer = std::ptr::null_mut();
                let status_code = unsafe {
                    napi::sys::napi_create_arraybuffer(
                        raw_env,
                        len,
                        &mut arraybuffer_data,
                        &mut arraybuffer,
                    )
                };
                if status_code == napi::sys::Status::napi_ok {
                    if len > 0 {
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                rust_call_status.error_buf_data,
                                arraybuffer_data as *mut u8,
                                len,
                            );
                        }
                    }

                    let mut typedarray = std::ptr::null_mut();
                    let status_code = unsafe {
                        napi::sys::napi_create_typedarray(
                            raw_env,
                            1, // napi_uint8_array
                            len,
                            arraybuffer,
                            0,
                            &mut typedarray,
                        )
                    };
                    if status_code == napi::sys::Status::napi_ok {
                        let js_uint8array = unsafe { JsUnknown::from_raw(raw_env, typedarray)? };
                        js_status.set_named_property("errorBuf", js_uint8array)?;
                    }
                }

                // Free the error_buf via rustbuffer_free
                if !rb_free_ptr.is_null() {
                    let error_rb = RustBufferC {
                        capacity: rust_call_status.error_buf_capacity,
                        len: rust_call_status.error_buf_len,
                        data: rust_call_status.error_buf_data,
                    };
                    let free_fn: RustBufferFreeFn = unsafe { std::mem::transmute(rb_free_ptr) };
                    let mut free_status = RustCallStatusC::default();
                    unsafe { free_fn(error_rb, &mut free_status as *mut RustCallStatusC) };
                }
            }
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
        data: if length > 0 {
            data_ptr
        } else {
            std::ptr::null()
        },
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
