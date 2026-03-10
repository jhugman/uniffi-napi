use napi::module_init;
use std::sync::OnceLock;
use std::thread::ThreadId;

mod ffi_type;

static MAIN_THREAD_ID: OnceLock<ThreadId> = OnceLock::new();

#[module_init]
fn init() {
    MAIN_THREAD_ID
        .set(std::thread::current().id())
        .expect("module_init called twice");
}

pub fn is_main_thread() -> bool {
    MAIN_THREAD_ID
        .get()
        .map(|id| *id == std::thread::current().id())
        .unwrap_or(false)
}
