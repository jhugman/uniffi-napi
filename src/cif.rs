//! Mapping from the abstract type system to libffi's concrete type system.
//!
//! This module is the bridge between [`FfiTypeDesc`](crate::ffi_type::FfiTypeDesc)
//! (our parsed, platform-independent type descriptions) and
//! [`libffi::middle::Type`] (the concrete ABI-level types that libffi uses to
//! build call-interface descriptors).
//!
//! The key design insight is that the mapping is surprisingly flat:
//!
//! - **Scalars** (`UInt8`..`Float64`, `Handle`) map one-to-one to libffi primitives.
//! - **`RustBuffer`** is the only pass-by-value struct. Its layout is
//!   `{ u64, u64, pointer }` — a three-field `Type::structure`.
//! - **Everything pointer-shaped** — `Reference`, `MutReference`, `Callback`,
//!   `VoidPointer`, and `RustCallStatus` (always passed as `&mut`) — collapses
//!   to a single `Type::pointer()` at the ABI level, regardless of what the
//!   pointer points to.
//! - **`ForeignBytes`** and bare **`Struct`** are intentionally unsupported:
//!   they are parseable from JS for completeness but never appear in actual
//!   UniFFI function signatures.

use libffi::middle::Type;

use crate::ffi_type::FfiTypeDesc;

/// Maps an [`FfiTypeDesc`] to a [`libffi::middle::Type`] suitable for CIF construction.
///
/// This is the single point of truth for how our abstract types become ABI types.
///
/// # Panics
///
/// Panics on `ForeignBytes` and bare `Struct(name)`, which are parseable from JS
/// but have no direct CIF representation. `Struct` is always used via
/// `Reference(Struct(...))`, which maps to `Type::pointer()`.
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
