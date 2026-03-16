use std::ffi::c_void;

use crate::ffi_c_types::{RustBufferC, RustBufferFreeFn, RustCallStatusC};

/// Create a JS Uint8Array from raw bytes.
///
/// Allocates a new ArrayBuffer, copies `len` bytes from `data` into it,
/// and returns a Uint8Array view. Returns the raw napi_value for the Uint8Array.
///
/// # Safety
/// `raw_env` must be a valid napi_env. `data` must point to at least `len` readable bytes
/// (ignored if `len == 0`).
pub unsafe fn create_uint8array(
    raw_env: napi::sys::napi_env,
    data: *const u8,
    len: usize,
) -> napi::Result<napi::sys::napi_value> {
    let mut arraybuffer_data: *mut c_void = std::ptr::null_mut();
    let mut arraybuffer = std::ptr::null_mut();
    let status =
        napi::sys::napi_create_arraybuffer(raw_env, len, &mut arraybuffer_data, &mut arraybuffer);
    if status != napi::sys::Status::napi_ok {
        return Err(napi::Error::from_reason("Failed to create ArrayBuffer"));
    }

    if len > 0 && !data.is_null() {
        std::ptr::copy_nonoverlapping(data, arraybuffer_data as *mut u8, len);
    }

    let mut typedarray = std::ptr::null_mut();
    let status = napi::sys::napi_create_typedarray(
        raw_env,
        napi::sys::TypedarrayType::uint8_array,
        len,
        arraybuffer,
        0,
        &mut typedarray,
    );
    if status != napi::sys::Status::napi_ok {
        return Err(napi::Error::from_reason("Failed to create Uint8Array"));
    }

    Ok(typedarray)
}

/// Read the data pointer and byte length from a JS TypedArray.
///
/// Returns `Some((data_ptr, length))` on success, `None` if the value is not a typed array.
///
/// # Safety
/// `raw_env` must be a valid napi_env. `raw_val` must be a valid napi_value.
pub unsafe fn read_typedarray_data(
    raw_env: napi::sys::napi_env,
    raw_val: napi::sys::napi_value,
) -> Option<(*const u8, usize)> {
    let mut length: usize = 0;
    let mut data: *mut c_void = std::ptr::null_mut();
    let mut ab = std::ptr::null_mut();
    let mut byte_offset: usize = 0;
    let mut ta_type: i32 = 0;
    let status = napi::sys::napi_get_typedarray_info(
        raw_env,
        raw_val,
        &mut ta_type,
        &mut length,
        &mut data,
        &mut ab,
        &mut byte_offset,
    );
    if status != napi::sys::Status::napi_ok {
        return None;
    }
    Some((data as *const u8, length))
}

/// Free a RustBufferC via the provided free function pointer.
///
/// No-op if the buffer's data is null, capacity is 0, or the free pointer is null.
///
/// # Safety
/// `free_ptr` must point to a valid `rustbuffer_free` function, or be null.
pub unsafe fn free_rustbuffer(rb: RustBufferC, free_ptr: *const c_void) {
    if !rb.data.is_null() && rb.capacity > 0 && !free_ptr.is_null() {
        let free_fn: RustBufferFreeFn = std::mem::transmute(free_ptr);
        let mut status = RustCallStatusC::default();
        free_fn(rb, &mut status as *mut _);
    }
}
