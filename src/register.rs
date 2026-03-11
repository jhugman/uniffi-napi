use std::ffi::c_void;
use std::sync::Arc;

use libffi::middle::{arg, Arg, Cif, CodePtr};
use napi::bindgen_prelude::*;
use napi::{JsObject, JsUnknown, Result};

use crate::cif::ffi_type_for;
use crate::ffi_type::FfiTypeDesc;
use crate::library::LibraryHandle;
use crate::marshal;

/// C layout of RustCallStatus, matching the test_lib definition.
#[repr(C)]
struct RustCallStatusC {
    code: i8,
    // RustBuffer: capacity, len, data
    error_buf_capacity: u64,
    error_buf_len: u64,
    error_buf_data: *mut u8,
}

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

        let js_func = env.create_function_from_closure(&name, move |ctx| {
            call_ffi_function(
                ctx.env,
                &ctx,
                &cif,
                symbol_ptr,
                &arg_types,
                &ret_type_clone,
                has_rust_call_status,
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
) -> Result<JsUnknown> {
    let declared_arg_count = arg_types.len();

    // Marshal JS arguments to boxed Rust values
    let mut boxed_args: Vec<Box<dyn std::any::Any>> = Vec::with_capacity(declared_arg_count);
    for (i, desc) in arg_types.iter().enumerate() {
        let js_val: JsUnknown = ctx.get(i)?;
        boxed_args.push(marshal::js_to_boxed(env, desc, js_val)?);
    }

    // Build libffi Arg references
    let mut ffi_args: Vec<Arg> = Vec::with_capacity(declared_arg_count + 1);
    for (i, desc) in arg_types.iter().enumerate() {
        ffi_args.push(marshal::boxed_to_arg(desc, boxed_args[i].as_ref()));
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
    marshal::ret_to_js(env, ret_type, ret_val.as_ref())
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
            _ => Err(napi::Error::from_reason(format!(
                "Unsupported return type: {:?}",
                ret_type
            ))),
        }
    }
}
