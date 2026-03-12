use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

#[repr(C)]
pub struct RustBuffer {
    pub capacity: u64,
    pub len: u64,
    pub data: *mut u8,
}

#[repr(C)]
pub struct ForeignBytes {
    pub len: i32,
    pub data: *const u8,
}

#[repr(C)]
pub struct RustCallStatus {
    pub code: i8,
    pub error_buf: RustBuffer,
}

// --- Scalar functions ---

#[no_mangle]
pub extern "C" fn uniffi_test_fn_add(a: i32, b: i32, status: &mut RustCallStatus) -> i32 {
    status.code = 0;
    a + b
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_negate(x: i8, status: &mut RustCallStatus) -> i8 {
    status.code = 0;
    -x
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_handle(status: &mut RustCallStatus) -> u64 {
    status.code = 0;
    0xDEAD_BEEF_1234_5678u64
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_void(status: &mut RustCallStatus) {
    status.code = 0;
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_double(x: f64, status: &mut RustCallStatus) -> f64 {
    status.code = 0;
    x * 2.0
}

// --- RustBuffer helpers ---

fn free_buffer(buf: RustBuffer) {
    if !buf.data.is_null() && buf.capacity > 0 {
        let layout = std::alloc::Layout::from_size_align(buf.capacity as usize, 1).unwrap();
        unsafe { std::alloc::dealloc(buf.data, layout) };
    }
}

// --- RustBuffer management ---

#[no_mangle]
pub extern "C" fn uniffi_test_rustbuffer_alloc(
    size: u64,
    status: &mut RustCallStatus,
) -> RustBuffer {
    status.code = 0;
    let layout = std::alloc::Layout::from_size_align(size as usize, 1).unwrap();
    let data = unsafe { std::alloc::alloc_zeroed(layout) };
    RustBuffer {
        capacity: size,
        len: 0,
        data,
    }
}

#[no_mangle]
pub extern "C" fn uniffi_test_rustbuffer_free(buf: RustBuffer, status: &mut RustCallStatus) {
    status.code = 0;
    free_buffer(buf);
}

#[no_mangle]
pub extern "C" fn uniffi_test_rustbuffer_from_bytes(
    bytes: ForeignBytes,
    status: &mut RustCallStatus,
) -> RustBuffer {
    status.code = 0;
    let len = bytes.len as usize;
    if len == 0 || bytes.data.is_null() {
        return RustBuffer {
            capacity: 0,
            len: 0,
            data: ptr::null_mut(),
        };
    }
    let layout = std::alloc::Layout::from_size_align(len, 1).unwrap();
    let data = unsafe {
        let ptr = std::alloc::alloc(layout);
        ptr::copy_nonoverlapping(bytes.data, ptr, len);
        ptr
    };
    RustBuffer {
        capacity: len as u64,
        len: len as u64,
        data,
    }
}

// --- RustBuffer echo (takes buffer, returns same bytes) ---

#[no_mangle]
pub extern "C" fn uniffi_test_fn_echo_buffer(
    buf: RustBuffer,
    status: &mut RustCallStatus,
) -> RustBuffer {
    status.code = 0;
    let len = buf.len as usize;
    if len == 0 || buf.data.is_null() {
        return RustBuffer {
            capacity: 0,
            len: 0,
            data: ptr::null_mut(),
        };
    }
    let layout = std::alloc::Layout::from_size_align(len, 1).unwrap();
    let new_data = unsafe {
        let ptr = std::alloc::alloc(layout);
        ptr::copy_nonoverlapping(buf.data, ptr, len);
        ptr
    };
    free_buffer(buf);
    RustBuffer {
        capacity: len as u64,
        len: len as u64,
        data: new_data,
    }
}

// --- RustBuffer multi-arg and utility functions ---

#[no_mangle]
pub extern "C" fn uniffi_test_fn_concat_buffers(
    buf1: RustBuffer,
    buf2: RustBuffer,
    status: &mut RustCallStatus,
) -> RustBuffer {
    status.code = 0;
    let len1 = buf1.len as usize;
    let len2 = buf2.len as usize;
    let total = len1 + len2;

    if total == 0 {
        free_buffer(buf1);
        free_buffer(buf2);
        return RustBuffer { capacity: 0, len: 0, data: ptr::null_mut() };
    }

    let layout = std::alloc::Layout::from_size_align(total, 1).unwrap();
    let data = unsafe {
        let ptr = std::alloc::alloc(layout);
        if len1 > 0 && !buf1.data.is_null() {
            ptr::copy_nonoverlapping(buf1.data, ptr, len1);
        }
        if len2 > 0 && !buf2.data.is_null() {
            ptr::copy_nonoverlapping(buf2.data, ptr.add(len1), len2);
        }
        ptr
    };

    free_buffer(buf1);
    free_buffer(buf2);

    RustBuffer { capacity: total as u64, len: total as u64, data }
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_buffer_len(
    buf: RustBuffer,
    status: &mut RustCallStatus,
) -> u32 {
    status.code = 0;
    let len = buf.len as u32;
    free_buffer(buf);
    len
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_make_buffer(
    value: u8,
    count: u32,
    status: &mut RustCallStatus,
) -> RustBuffer {
    status.code = 0;
    let len = count as usize;
    if len == 0 {
        return RustBuffer { capacity: 0, len: 0, data: ptr::null_mut() };
    }
    let layout = std::alloc::Layout::from_size_align(len, 1).unwrap();
    let data = unsafe {
        let ptr = std::alloc::alloc(layout);
        ptr::write_bytes(ptr, value, len);
        ptr
    };
    RustBuffer { capacity: len as u64, len: len as u64, data }
}

// --- Error-producing function ---

#[no_mangle]
pub extern "C" fn uniffi_test_fn_error(status: &mut RustCallStatus) -> i32 {
    let msg = b"something went wrong";
    let len = msg.len();
    let layout = std::alloc::Layout::from_size_align(len, 1).unwrap();
    let data = unsafe {
        let ptr = std::alloc::alloc(layout);
        ptr::copy_nonoverlapping(msg.as_ptr(), ptr, len);
        ptr
    };
    status.code = 2; // CALL_UNEXPECTED_ERROR
    status.error_buf = RustBuffer {
        capacity: len as u64,
        len: len as u64,
        data,
    };
    0
}

// --- Callback test: calls a function pointer ---

pub type SimpleCallback = extern "C" fn(u64, i8);

#[no_mangle]
pub extern "C" fn uniffi_test_fn_call_callback(
    cb: SimpleCallback,
    handle: u64,
    value: i8,
    status: &mut RustCallStatus,
) {
    status.code = 0;
    cb(handle, value);
}

// --- Callback with RustBuffer arg ---

pub type BufferCallback = extern "C" fn(u64, RustBuffer);

#[no_mangle]
pub extern "C" fn uniffi_test_fn_call_callback_with_buffer(
    cb: BufferCallback,
    handle: u64,
    status: &mut RustCallStatus,
) {
    status.code = 0;
    // Create a buffer with known content [0xDE, 0xAD, 0xBE, 0xEF]
    let data_bytes: &[u8] = &[0xDE, 0xAD, 0xBE, 0xEF];
    let len = data_bytes.len();
    let layout = std::alloc::Layout::from_size_align(len, 1).unwrap();
    let data = unsafe {
        let ptr = std::alloc::alloc(layout);
        ptr::copy_nonoverlapping(data_bytes.as_ptr(), ptr, len);
        ptr
    };
    let buf = RustBuffer {
        capacity: len as u64,
        len: len as u64,
        data,
    };
    cb(handle, buf);
    // Callback takes ownership of the buffer — do NOT free here
}

// --- Callback from another thread ---

#[no_mangle]
pub extern "C" fn uniffi_test_fn_call_callback_from_thread(
    cb: SimpleCallback,
    handle: u64,
    value: i8,
    status: &mut RustCallStatus,
) {
    status.code = 0;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        cb(handle, value);
    });
}

// --- VTable test ---

#[repr(C)]
pub struct TestVTable {
    pub get_value: extern "C" fn(u64, &mut RustCallStatus) -> i32,
    pub free: extern "C" fn(u64, &mut RustCallStatus),
}

static mut STORED_VTABLE: Option<TestVTable> = None;

#[no_mangle]
pub extern "C" fn uniffi_test_fn_init_vtable(vtable: &TestVTable, status: &mut RustCallStatus) {
    status.code = 0;
    unsafe {
        STORED_VTABLE = Some(TestVTable {
            get_value: vtable.get_value,
            free: vtable.free,
        });
    }
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_use_vtable(handle: u64, status: &mut RustCallStatus) -> i32 {
    status.code = 0;
    unsafe {
        if let Some(ref vtable) = STORED_VTABLE {
            let mut cb_status = RustCallStatus {
                code: 0,
                error_buf: RustBuffer { capacity: 0, len: 0, data: std::ptr::null_mut() },
            };
            (vtable.get_value)(handle, &mut cb_status)
        } else {
            -1
        }
    }
}

// --- Blocking cross-thread test ---

static THREAD_RESULT: AtomicI32 = AtomicI32::new(0);
static THREAD_DONE: AtomicBool = AtomicBool::new(false);

#[no_mangle]
pub extern "C" fn uniffi_test_fn_use_vtable_from_thread(
    handle: u64,
    status: &mut RustCallStatus,
) {
    status.code = 0;
    THREAD_DONE.store(false, Ordering::SeqCst);
    unsafe {
        if let Some(ref vtable) = STORED_VTABLE {
            let get_value = vtable.get_value;
            std::thread::spawn(move || {
                let mut cb_status = RustCallStatus {
                    code: 0,
                    error_buf: RustBuffer {
                        capacity: 0,
                        len: 0,
                        data: std::ptr::null_mut(),
                    },
                };
                let result = (get_value)(handle, &mut cb_status);
                THREAD_RESULT.store(result, Ordering::SeqCst);
                THREAD_DONE.store(true, Ordering::SeqCst);
            });
        }
    }
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_is_thread_done(status: &mut RustCallStatus) -> i8 {
    status.code = 0;
    if THREAD_DONE.load(Ordering::SeqCst) { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_get_thread_result(status: &mut RustCallStatus) -> i32 {
    status.code = 0;
    THREAD_RESULT.load(Ordering::SeqCst)
}

// --- Non-blocking cross-thread test ---

#[repr(C)]
pub struct NotifyVTable {
    pub notify: extern "C" fn(u64),
}

static mut STORED_NOTIFY_VTABLE: Option<NotifyVTable> = None;
static NOTIFY_DONE: AtomicBool = AtomicBool::new(false);

#[no_mangle]
pub extern "C" fn uniffi_test_fn_init_notify_vtable(
    vtable: &NotifyVTable,
    status: &mut RustCallStatus,
) {
    status.code = 0;
    unsafe {
        STORED_NOTIFY_VTABLE = Some(NotifyVTable {
            notify: vtable.notify,
        });
    }
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_notify_from_thread(
    handle: u64,
    status: &mut RustCallStatus,
) {
    status.code = 0;
    NOTIFY_DONE.store(false, Ordering::SeqCst);
    unsafe {
        if let Some(ref vtable) = STORED_NOTIFY_VTABLE {
            let notify = vtable.notify;
            std::thread::spawn(move || {
                (notify)(handle);
                NOTIFY_DONE.store(true, Ordering::SeqCst);
            });
        }
    }
}

#[no_mangle]
pub extern "C" fn uniffi_test_fn_is_notify_done(status: &mut RustCallStatus) -> i8 {
    status.code = 0;
    if NOTIFY_DONE.load(Ordering::SeqCst) { 1 } else { 0 }
}
