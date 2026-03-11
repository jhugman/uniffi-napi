use napi::bindgen_prelude::*;
use napi::{JsObject, Result};

/// Mirrors uniffi_bindgen's FfiType enum.
/// Parsed from tagged JS objects like { tag: 'Int32' } or { tag: 'Callback', name: 'cb_name' }.
#[derive(Debug, Clone)]
pub enum FfiTypeDesc {
    UInt8,
    Int8,
    UInt16,
    Int16,
    UInt32,
    Int32,
    UInt64,
    Int64,
    Float32,
    Float64,
    Handle,
    RustBuffer,
    ForeignBytes,
    RustCallStatus,
    Callback(String),
    Struct(String),
    Reference(Box<FfiTypeDesc>),
    MutReference(Box<FfiTypeDesc>),
    VoidPointer,
    Void,
}

impl FfiTypeDesc {
    /// Parse from a JS object with { tag: string, ...params }.
    pub fn from_js_object(env: &Env, obj: &JsObject) -> Result<Self> {
        let tag: String = obj.get_named_property::<String>("tag")?;
        match tag.as_str() {
            "UInt8" => Ok(Self::UInt8),
            "Int8" => Ok(Self::Int8),
            "UInt16" => Ok(Self::UInt16),
            "Int16" => Ok(Self::Int16),
            "UInt32" => Ok(Self::UInt32),
            "Int32" => Ok(Self::Int32),
            "UInt64" => Ok(Self::UInt64),
            "Int64" => Ok(Self::Int64),
            "Float32" => Ok(Self::Float32),
            "Float64" => Ok(Self::Float64),
            "Handle" => Ok(Self::Handle),
            "RustBuffer" => Ok(Self::RustBuffer),
            "ForeignBytes" => Ok(Self::ForeignBytes),
            "RustCallStatus" => Ok(Self::RustCallStatus),
            "VoidPointer" => Ok(Self::VoidPointer),
            "Void" => Ok(Self::Void),
            "Callback" => {
                let name: String = obj.get_named_property::<String>("name")?;
                Ok(Self::Callback(name))
            }
            "Struct" => {
                let name: String = obj.get_named_property::<String>("name")?;
                Ok(Self::Struct(name))
            }
            "Reference" => {
                let inner: JsObject = obj.get_named_property("inner")?;
                Ok(Self::Reference(Box::new(Self::from_js_object(env, &inner)?)))
            }
            "MutReference" => {
                let inner: JsObject = obj.get_named_property("inner")?;
                Ok(Self::MutReference(Box::new(Self::from_js_object(env, &inner)?)))
            }
            other => Err(napi::Error::from_reason(format!(
                "Unknown FfiType tag: {other}"
            ))),
        }
    }
}
