use std::ffi::c_void;

use libffi::low;
use libffi::middle::{Cif, Type};
use napi::threadsafe_function::{ErrorStrategy, ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Env, NapiRaw, NapiValue};

use crate::cif::ffi_type_for;
use crate::ffi_type::FfiTypeDesc;
use crate::is_main_thread;

/// Definition of a callback parsed from the JS `callbacks` map.
#[derive(Debug, Clone)]
pub struct CallbackDef {
    pub args: Vec<FfiTypeDesc>,
    pub ret: FfiTypeDesc,
    pub has_rust_call_status: bool,
}

/// Captured C argument values for cross-thread dispatch.
/// These are read from raw pointers on the calling thread, then sent to the
/// main thread where they are converted to JS values.
#[derive(Clone, Debug)]
pub enum RawCallbackArg {
    UInt8(u8),
    Int8(i8),
    UInt16(u16),
    Int16(i16),
    UInt32(u32),
    Int32(i32),
    UInt64(u64),
    Int64(i64),
    Float32(f32),
    Float64(f64),
}

/// Userdata passed to the libffi closure trampoline.
/// Holds raw napi pointers so we can reconstruct Env and call the JS function.
/// Also holds an optional ThreadsafeFunction for cross-thread dispatch.
pub struct TrampolineUserdata {
    pub raw_env: napi::sys::napi_env,
    pub raw_fn: napi::sys::napi_value,
    pub arg_types: Vec<FfiTypeDesc>,
    pub tsfn: Option<ThreadsafeFunction<Vec<RawCallbackArg>, ErrorStrategy::Fatal>>,
    /// Pointer to rustbuffer_free. Needed to free RustBuffer args passed to this callback.
    /// We don't need rb_from_bytes_ptr because simple callbacks never return RustBuffers.
    pub rb_free_ptr: *const c_void,
}

// Safety: The ThreadsafeFunction is designed to be used across threads.
// The raw_env and raw_fn are only accessed on the main thread path.
unsafe impl Send for TrampolineUserdata {}
unsafe impl Sync for TrampolineUserdata {}

/// The trampoline callback invoked by libffi when C code calls the function pointer.
///
/// Safety: This function is called from C via libffi. It reconstructs the napi Env
/// from the raw pointer stored in userdata, reads the C arguments, marshals them to
/// JS values, and calls the JS callback function.
///
/// If called from a non-main thread, it serializes the arguments and dispatches
/// via ThreadsafeFunction to the Node.js event loop.
pub unsafe extern "C" fn trampoline_callback(
    _cif: &low::ffi_cif,
    _result: &mut c_void,
    args: *const *const c_void,
    userdata: &TrampolineUserdata,
) {
    if is_main_thread() {
        // Same-thread path: call JS function directly
        trampoline_main_thread(_cif, _result, args, userdata);
    } else {
        // Cross-thread path: serialize args and dispatch via TSF
        trampoline_cross_thread(args, userdata);
    }
}

/// Direct JS call on the main thread (existing behavior).
unsafe fn trampoline_main_thread(
    _cif: &low::ffi_cif,
    _result: &mut c_void,
    args: *const *const c_void,
    userdata: &TrampolineUserdata,
) {
    let env = Env::from_raw(userdata.raw_env);

    let js_fn = match napi::JsFunction::from_raw(userdata.raw_env, userdata.raw_fn) {
        Ok(f) => f,
        Err(_) => return,
    };

    let arg_count = userdata.arg_types.len();

    let mut js_args: Vec<napi::JsUnknown> = Vec::with_capacity(arg_count);
    for (i, desc) in userdata.arg_types.iter().enumerate() {
        let arg_ptr = *args.add(i);
        let js_val = match c_arg_to_js(&env, desc, arg_ptr) {
            Ok(v) => v,
            Err(_) => return,
        };
        js_args.push(js_val);
    }

    let _result = js_fn.call(None, &js_args);
}

/// Cross-thread dispatch: read C args into RawCallbackArg values and send via TSF.
unsafe fn trampoline_cross_thread(args: *const *const c_void, userdata: &TrampolineUserdata) {
    let tsfn = match &userdata.tsfn {
        Some(t) => t,
        None => return, // No TSF available, can't dispatch
    };

    let mut raw_args = Vec::with_capacity(userdata.arg_types.len());
    for (i, desc) in userdata.arg_types.iter().enumerate() {
        let arg_ptr = *args.add(i);
        let raw_arg = match read_raw_arg(desc, arg_ptr) {
            Some(a) => a,
            None => return,
        };
        raw_args.push(raw_arg);
    }

    // Dispatch to main thread. NonBlocking means we don't wait for the result.
    tsfn.call(raw_args, ThreadsafeFunctionCallMode::NonBlocking);
}

/// Read a C argument from a raw pointer into a RawCallbackArg.
pub unsafe fn read_raw_arg(desc: &FfiTypeDesc, arg_ptr: *const c_void) -> Option<RawCallbackArg> {
    match desc {
        FfiTypeDesc::UInt8 => Some(RawCallbackArg::UInt8(*(arg_ptr as *const u8))),
        FfiTypeDesc::Int8 => Some(RawCallbackArg::Int8(*(arg_ptr as *const i8))),
        FfiTypeDesc::UInt16 => Some(RawCallbackArg::UInt16(*(arg_ptr as *const u16))),
        FfiTypeDesc::Int16 => Some(RawCallbackArg::Int16(*(arg_ptr as *const i16))),
        FfiTypeDesc::UInt32 => Some(RawCallbackArg::UInt32(*(arg_ptr as *const u32))),
        FfiTypeDesc::Int32 => Some(RawCallbackArg::Int32(*(arg_ptr as *const i32))),
        FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => {
            Some(RawCallbackArg::UInt64(*(arg_ptr as *const u64)))
        }
        FfiTypeDesc::Int64 => Some(RawCallbackArg::Int64(*(arg_ptr as *const i64))),
        FfiTypeDesc::Float32 => Some(RawCallbackArg::Float32(*(arg_ptr as *const f32))),
        FfiTypeDesc::Float64 => Some(RawCallbackArg::Float64(*(arg_ptr as *const f64))),
        _ => None,
    }
}

/// Convert a RawCallbackArg to a JS value on the main thread.
pub fn raw_arg_to_js(env: &Env, raw_arg: &RawCallbackArg) -> napi::Result<napi::JsUnknown> {
    match raw_arg {
        RawCallbackArg::UInt8(v) => Ok(env.create_uint32(*v as u32)?.into_unknown()),
        RawCallbackArg::Int8(v) => Ok(env.create_int32(*v as i32)?.into_unknown()),
        RawCallbackArg::UInt16(v) => Ok(env.create_uint32(*v as u32)?.into_unknown()),
        RawCallbackArg::Int16(v) => Ok(env.create_int32(*v as i32)?.into_unknown()),
        RawCallbackArg::UInt32(v) => Ok(env.create_uint32(*v)?.into_unknown()),
        RawCallbackArg::Int32(v) => Ok(env.create_int32(*v)?.into_unknown()),
        RawCallbackArg::UInt64(v) => Ok(env.create_bigint_from_u64(*v)?.into_unknown()?),
        RawCallbackArg::Int64(v) => Ok(env.create_bigint_from_i64(*v)?.into_unknown()?),
        RawCallbackArg::Float32(v) => Ok(env.create_double(*v as f64)?.into_unknown()),
        RawCallbackArg::Float64(v) => Ok(env.create_double(*v)?.into_unknown()),
    }
}

/// Convert a JS return value to a RawCallbackArg based on the expected return type.
/// Used by VTable cross-thread dispatch to send return values back to the calling thread.
pub fn js_return_to_raw(
    env: &Env,
    ret_type: &FfiTypeDesc,
    js_val: napi::JsUnknown,
) -> Option<RawCallbackArg> {
    let raw_env = env.raw();
    unsafe {
        match ret_type {
            FfiTypeDesc::Int8 => {
                let num = napi::JsNumber::from_raw(raw_env, js_val.raw()).ok()?;
                Some(RawCallbackArg::Int8(num.get_double().ok()? as i8))
            }
            FfiTypeDesc::UInt8 => {
                let num = napi::JsNumber::from_raw(raw_env, js_val.raw()).ok()?;
                Some(RawCallbackArg::UInt8(num.get_double().ok()? as u8))
            }
            FfiTypeDesc::Int16 => {
                let num = napi::JsNumber::from_raw(raw_env, js_val.raw()).ok()?;
                Some(RawCallbackArg::Int16(num.get_double().ok()? as i16))
            }
            FfiTypeDesc::UInt16 => {
                let num = napi::JsNumber::from_raw(raw_env, js_val.raw()).ok()?;
                Some(RawCallbackArg::UInt16(num.get_double().ok()? as u16))
            }
            FfiTypeDesc::Int32 => {
                let num = napi::JsNumber::from_raw(raw_env, js_val.raw()).ok()?;
                Some(RawCallbackArg::Int32(num.get_double().ok()? as i32))
            }
            FfiTypeDesc::UInt32 => {
                let num = napi::JsNumber::from_raw(raw_env, js_val.raw()).ok()?;
                Some(RawCallbackArg::UInt32(num.get_double().ok()? as u32))
            }
            FfiTypeDesc::Int64 => {
                let bigint = napi::JsBigInt::from_raw(raw_env, js_val.raw()).ok()?;
                let (v, _) = bigint.get_i64().ok()?;
                Some(RawCallbackArg::Int64(v))
            }
            FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => {
                let bigint = napi::JsBigInt::from_raw(raw_env, js_val.raw()).ok()?;
                let (v, _) = bigint.get_u64().ok()?;
                Some(RawCallbackArg::UInt64(v))
            }
            FfiTypeDesc::Float32 => {
                let num = napi::JsNumber::from_raw(raw_env, js_val.raw()).ok()?;
                Some(RawCallbackArg::Float32(num.get_double().ok()? as f32))
            }
            FfiTypeDesc::Float64 => {
                let num = napi::JsNumber::from_raw(raw_env, js_val.raw()).ok()?;
                Some(RawCallbackArg::Float64(num.get_double().ok()?))
            }
            _ => None,
        }
    }
}

/// Read a C argument from a raw pointer and convert it to a JS value.
pub unsafe fn c_arg_to_js(
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
