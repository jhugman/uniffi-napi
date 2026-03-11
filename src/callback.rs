use std::ffi::c_void;

use libffi::low;
use libffi::middle::{Cif, Type};
use napi::{Env, NapiValue};

use crate::cif::ffi_type_for;
use crate::ffi_type::FfiTypeDesc;

/// Definition of a callback parsed from the JS `callbacks` map.
#[derive(Debug, Clone)]
pub struct CallbackDef {
    pub args: Vec<FfiTypeDesc>,
    pub ret: FfiTypeDesc,
    pub has_rust_call_status: bool,
}

/// Userdata passed to the libffi closure trampoline.
/// Holds raw napi pointers so we can reconstruct Env and call the JS function.
/// This is safe only for same-thread callbacks (the closure is called synchronously
/// on the same thread that created it).
pub struct TrampolineUserdata {
    pub raw_env: napi::sys::napi_env,
    pub raw_fn: napi::sys::napi_value,
    pub arg_types: Vec<FfiTypeDesc>,
}

/// The trampoline callback invoked by libffi when C code calls the function pointer.
///
/// Safety: This function is called from C via libffi. It reconstructs the napi Env
/// from the raw pointer stored in userdata, reads the C arguments, marshals them to
/// JS values, and calls the JS callback function.
pub unsafe extern "C" fn trampoline_callback(
    _cif: &low::ffi_cif,
    _result: &mut c_void,
    args: *const *const c_void,
    userdata: &TrampolineUserdata,
) {
    // Reconstruct the Env from the raw pointer.
    let env = Env::from_raw(userdata.raw_env);

    // Reconstruct the JS function from the raw napi_value.
    let js_fn = match napi::JsFunction::from_raw(userdata.raw_env, userdata.raw_fn) {
        Ok(f) => f,
        Err(_) => return,
    };

    let arg_count = userdata.arg_types.len();

    // Marshal each C argument to a JS value
    let mut js_args: Vec<napi::JsUnknown> = Vec::with_capacity(arg_count);
    for (i, desc) in userdata.arg_types.iter().enumerate() {
        let arg_ptr = *args.offset(i as isize);
        let js_val = match c_arg_to_js(&env, desc, arg_ptr) {
            Ok(v) => v,
            Err(_) => return,
        };
        js_args.push(js_val);
    }

    // Call the JS function with no `this` binding
    let _result = js_fn.call(None, &js_args);
}

/// Read a C argument from a raw pointer and convert it to a JS value.
unsafe fn c_arg_to_js(
    env: &Env,
    desc: &FfiTypeDesc,
    arg_ptr: *const c_void,
) -> napi::Result<napi::JsUnknown> {
    match desc {
        FfiTypeDesc::UInt8 => {
            let v = *(arg_ptr as *const u8);
            Ok(env.create_uint32(v as u32)?.into_unknown())
        }
        FfiTypeDesc::Int8 => {
            let v = *(arg_ptr as *const i8);
            Ok(env.create_int32(v as i32)?.into_unknown())
        }
        FfiTypeDesc::UInt16 => {
            let v = *(arg_ptr as *const u16);
            Ok(env.create_uint32(v as u32)?.into_unknown())
        }
        FfiTypeDesc::Int16 => {
            let v = *(arg_ptr as *const i16);
            Ok(env.create_int32(v as i32)?.into_unknown())
        }
        FfiTypeDesc::UInt32 => {
            let v = *(arg_ptr as *const u32);
            Ok(env.create_uint32(v)?.into_unknown())
        }
        FfiTypeDesc::Int32 => {
            let v = *(arg_ptr as *const i32);
            Ok(env.create_int32(v)?.into_unknown())
        }
        FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => {
            let v = *(arg_ptr as *const u64);
            Ok(env.create_bigint_from_u64(v)?.into_unknown()?)
        }
        FfiTypeDesc::Int64 => {
            let v = *(arg_ptr as *const i64);
            Ok(env.create_bigint_from_i64(v)?.into_unknown()?)
        }
        FfiTypeDesc::Float32 => {
            let v = *(arg_ptr as *const f32);
            Ok(env.create_double(v as f64)?.into_unknown())
        }
        FfiTypeDesc::Float64 => {
            let v = *(arg_ptr as *const f64);
            Ok(env.create_double(v)?.into_unknown())
        }
        _ => Err(napi::Error::from_reason(format!(
            "Unsupported callback arg type: {:?}",
            desc
        ))),
    }
}

/// Build a CIF for a callback definition.
pub fn build_callback_cif(callback_def: &CallbackDef) -> Cif {
    let cif_arg_types: Vec<Type> = callback_def.args.iter().map(ffi_type_for).collect();
    let cif_ret_type = ffi_type_for(&callback_def.ret);
    Cif::new(cif_arg_types, cif_ret_type)
}
