use libffi::middle::Type;

use crate::ffi_type::FfiTypeDesc;

/// Maps an `FfiTypeDesc` to a `libffi::middle::Type`.
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
        FfiTypeDesc::ForeignBytes => todo!("ForeignBytes struct type"),
        FfiTypeDesc::Struct(_) => todo!("Custom struct type - Task 10"),
    }
}
