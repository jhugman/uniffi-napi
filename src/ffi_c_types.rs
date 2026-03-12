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
pub(crate) struct ForeignBytesC {
    pub len: i32,
    pub data: *const u8,
}

/// C layout of RustCallStatus, matching the UniFFI convention.
#[repr(C)]
pub(crate) struct RustCallStatusC {
    pub code: i8,
    // RustBuffer: capacity, len, data (inlined to match C layout)
    pub error_buf_capacity: u64,
    pub error_buf_len: u64,
    pub error_buf_data: *mut u8,
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

pub(crate) type RustBufferFromBytesFn =
    unsafe extern "C" fn(ForeignBytesC, *mut RustCallStatusC) -> RustBufferC;
pub(crate) type RustBufferFreeFn = unsafe extern "C" fn(RustBufferC, *mut RustCallStatusC);
