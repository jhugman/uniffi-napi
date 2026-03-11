use std::any::Any;

use libffi::middle::{Arg, arg};
use napi::bindgen_prelude::*;
use napi::{JsBigInt, JsNumber, JsUnknown, NapiRaw, NapiValue, Result};

use crate::ffi_type::FfiTypeDesc;

/// Convert a JS value to a boxed Rust value matching the given FFI type.
/// Returns a Box<dyn Any> that holds the properly typed value.
pub fn js_to_boxed(env: &Env, desc: &FfiTypeDesc, js_val: JsUnknown) -> Result<Box<dyn Any>> {
    match desc {
        FfiTypeDesc::UInt8 => {
            let n: JsNumber = js_val.try_into()?;
            let v: f64 = n.get_double()?;
            Ok(Box::new(v as u8))
        }
        FfiTypeDesc::Int8 => {
            let n: JsNumber = js_val.try_into()?;
            let v: f64 = n.get_double()?;
            Ok(Box::new(v as i8))
        }
        FfiTypeDesc::UInt16 => {
            let n: JsNumber = js_val.try_into()?;
            let v: f64 = n.get_double()?;
            Ok(Box::new(v as u16))
        }
        FfiTypeDesc::Int16 => {
            let n: JsNumber = js_val.try_into()?;
            let v: f64 = n.get_double()?;
            Ok(Box::new(v as i16))
        }
        FfiTypeDesc::UInt32 => {
            let n: JsNumber = js_val.try_into()?;
            let v: f64 = n.get_double()?;
            Ok(Box::new(v as u32))
        }
        FfiTypeDesc::Int32 => {
            let n: JsNumber = js_val.try_into()?;
            let v: f64 = n.get_double()?;
            Ok(Box::new(v as i32))
        }
        FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => {
            let bigint = unsafe { JsBigInt::from_raw(env.raw(), js_val.raw())? };
            let (v, _lossless) = bigint.get_u64()?;
            Ok(Box::new(v))
        }
        FfiTypeDesc::Int64 => {
            let bigint = unsafe { JsBigInt::from_raw(env.raw(), js_val.raw())? };
            let (v, _lossless) = bigint.get_i64()?;
            Ok(Box::new(v))
        }
        FfiTypeDesc::Float32 => {
            let n: JsNumber = js_val.try_into()?;
            let v: f64 = n.get_double()?;
            Ok(Box::new(v as f32))
        }
        FfiTypeDesc::Float64 => {
            let n: JsNumber = js_val.try_into()?;
            let v: f64 = n.get_double()?;
            Ok(Box::new(v))
        }
        _ => Err(napi::Error::from_reason(format!(
            "Unsupported argument type for js_to_boxed: {:?}",
            desc
        ))),
    }
}

/// Create a libffi `Arg` from a boxed value and its type descriptor.
/// The Arg borrows from the boxed value, so the box must outlive the Arg.
pub fn boxed_to_arg<'a>(desc: &FfiTypeDesc, boxed: &'a dyn Any) -> Arg<'a> {
    match desc {
        FfiTypeDesc::UInt8 => arg(boxed.downcast_ref::<u8>().unwrap()),
        FfiTypeDesc::Int8 => arg(boxed.downcast_ref::<i8>().unwrap()),
        FfiTypeDesc::UInt16 => arg(boxed.downcast_ref::<u16>().unwrap()),
        FfiTypeDesc::Int16 => arg(boxed.downcast_ref::<i16>().unwrap()),
        FfiTypeDesc::UInt32 => arg(boxed.downcast_ref::<u32>().unwrap()),
        FfiTypeDesc::Int32 => arg(boxed.downcast_ref::<i32>().unwrap()),
        FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => arg(boxed.downcast_ref::<u64>().unwrap()),
        FfiTypeDesc::Int64 => arg(boxed.downcast_ref::<i64>().unwrap()),
        FfiTypeDesc::Float32 => arg(boxed.downcast_ref::<f32>().unwrap()),
        FfiTypeDesc::Float64 => arg(boxed.downcast_ref::<f64>().unwrap()),
        FfiTypeDesc::RustCallStatus => {
            // The boxed value is a *mut RustCallStatusC
            arg(boxed.downcast_ref::<*mut u8>().unwrap())
        }
        _ => panic!("Unsupported argument type for boxed_to_arg: {:?}", desc),
    }
}

/// Convert a raw return value to a JS value.
/// This is called after cif.call() with the correct return type R.
pub fn ret_to_js(env: &Env, desc: &FfiTypeDesc, boxed: &dyn Any) -> Result<JsUnknown> {
    match desc {
        FfiTypeDesc::Void => Ok(env.get_undefined()?.into_unknown()),
        FfiTypeDesc::UInt8 => {
            let v = boxed.downcast_ref::<u8>().unwrap();
            Ok(env.create_uint32(*v as u32)?.into_unknown())
        }
        FfiTypeDesc::Int8 => {
            let v = boxed.downcast_ref::<i8>().unwrap();
            Ok(env.create_int32(*v as i32)?.into_unknown())
        }
        FfiTypeDesc::UInt16 => {
            let v = boxed.downcast_ref::<u16>().unwrap();
            Ok(env.create_uint32(*v as u32)?.into_unknown())
        }
        FfiTypeDesc::Int16 => {
            let v = boxed.downcast_ref::<i16>().unwrap();
            Ok(env.create_int32(*v as i32)?.into_unknown())
        }
        FfiTypeDesc::UInt32 => {
            let v = boxed.downcast_ref::<u32>().unwrap();
            Ok(env.create_uint32(*v)?.into_unknown())
        }
        FfiTypeDesc::Int32 => {
            let v = boxed.downcast_ref::<i32>().unwrap();
            Ok(env.create_int32(*v)?.into_unknown())
        }
        FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => {
            let v = boxed.downcast_ref::<u64>().unwrap();
            Ok(env.create_bigint_from_u64(*v)?.into_unknown()?)
        }
        FfiTypeDesc::Int64 => {
            let v = boxed.downcast_ref::<i64>().unwrap();
            Ok(env.create_bigint_from_i64(*v)?.into_unknown()?)
        }
        FfiTypeDesc::Float32 => {
            let v = boxed.downcast_ref::<f32>().unwrap();
            Ok(env.create_double(*v as f64)?.into_unknown())
        }
        FfiTypeDesc::Float64 => {
            let v = boxed.downcast_ref::<f64>().unwrap();
            Ok(env.create_double(*v)?.into_unknown())
        }
        _ => Err(napi::Error::from_reason(format!(
            "Unsupported return type for ret_to_js: {:?}",
            desc
        ))),
    }
}
