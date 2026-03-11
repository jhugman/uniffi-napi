use napi::bindgen_prelude::*;
use napi::module_init;
use napi::JsObject;
use napi_derive::napi;
use std::sync::OnceLock;
use std::thread::ThreadId;

mod callback;
mod cif;
mod ffi_type;
mod library;
mod marshal;
mod register;

use library::LibraryHandle;

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

#[napi(object)]
pub struct RustBufferSymbols {
    pub rustbuffer_alloc: String,
    pub rustbuffer_free: String,
    pub rustbuffer_from_bytes: String,
}

#[napi]
pub struct UniffiNativeModule {
    handle: Option<LibraryHandle>,
}

#[napi]
impl UniffiNativeModule {
    #[napi(factory)]
    pub fn open(path: String, symbols: RustBufferSymbols) -> napi::Result<Self> {
        let handle = LibraryHandle::open(
            &path,
            &symbols.rustbuffer_alloc,
            &symbols.rustbuffer_free,
            &symbols.rustbuffer_from_bytes,
        )?;
        Ok(Self {
            handle: Some(handle),
        })
    }

    #[napi]
    pub fn close(&mut self) {
        self.handle.take();
    }

    #[napi]
    pub fn register(&self, env: Env, definitions: JsObject) -> napi::Result<JsObject> {
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| napi::Error::from_reason("Module is closed"))?;
        register::register(env, handle, definitions)
    }
}
