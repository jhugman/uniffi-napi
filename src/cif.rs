use libffi::middle::Type;

use crate::ffi_type::FfiTypeDesc;

/// Maps an `FfiTypeDesc` to a `libffi::middle::Type`.
///
/// Panics on unsupported types (`ForeignBytes`, bare `Struct`). These types
/// are parseable from JS but have no direct CIF representation. `Struct` is
/// always used via `Reference(Struct(...))` which maps to `Type::pointer()`.
pub fn ffi_type_for(desc: &FfiTypeDesc) -> Type {
    match desc {
        FfiTypeDesc::UInt8 => Type::u8(),
        FfiTypeDesc::Int8 => Type::i8(),
        FfiTypeDesc::UInt16 => Type::u16(),
        FfiTypeDesc::Int16 => Type::i16(),
        FfiTypeDesc::UInt32 => Type::u32(),
        FfiTypeDesc::Int32 => Type::i32(),
        FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => Type::u64(),
        FfiTypeDesc::Int64 => Type::i64(),
        FfiTypeDesc::Float32 => Type::f32(),
        FfiTypeDesc::Float64 => Type::f64(),
        FfiTypeDesc::VoidPointer
        | FfiTypeDesc::Reference(_)
        | FfiTypeDesc::MutReference(_)
        | FfiTypeDesc::Callback(_) => Type::pointer(),
        FfiTypeDesc::Void => Type::void(),
        FfiTypeDesc::RustCallStatus => Type::pointer(), // always passed as &mut
        FfiTypeDesc::RustBuffer => Type::structure(vec![Type::u64(), Type::u64(), Type::pointer()]),
        FfiTypeDesc::ForeignBytes => {
            panic!("ForeignBytes has no CIF representation; it is not used in UniFFI function signatures")
        }
        FfiTypeDesc::Struct(name) => {
            panic!("Bare Struct('{name}') has no CIF representation; use Reference(Struct('{name}')) for pass-by-pointer")
        }
    }
}
