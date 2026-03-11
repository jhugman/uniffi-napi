use dlopen2::raw::Library;
use napi::Result;
use std::ffi::c_void;

/// Holds a dlopen handle and pre-resolved rustbuffer management symbols.
#[allow(dead_code)]
pub struct LibraryHandle {
    pub lib: Library,
    pub rustbuffer_alloc: *const c_void,
    pub rustbuffer_free: *const c_void,
    pub rustbuffer_from_bytes: *const c_void,
}

// Safety: symbol pointers are valid for the lifetime of the Library handle.
unsafe impl Send for LibraryHandle {}

impl LibraryHandle {
    pub fn open(path: &str, alloc: &str, free: &str, from_bytes: &str) -> Result<Self> {
        let lib = Library::open(path)
            .map_err(|e| napi::Error::from_reason(format!("dlopen failed for '{path}': {e}")))?;

        let rustbuffer_alloc: *const c_void = unsafe {
            lib.symbol(alloc)
                .map_err(|e| napi::Error::from_reason(format!("Symbol '{alloc}' not found: {e}")))?
        };
        let rustbuffer_free: *const c_void = unsafe {
            lib.symbol(free)
                .map_err(|e| napi::Error::from_reason(format!("Symbol '{free}' not found: {e}")))?
        };
        let rustbuffer_from_bytes: *const c_void = unsafe {
            lib.symbol(from_bytes).map_err(|e| {
                napi::Error::from_reason(format!("Symbol '{from_bytes}' not found: {e}"))
            })?
        };

        Ok(Self {
            lib,
            rustbuffer_alloc,
            rustbuffer_free,
            rustbuffer_from_bytes,
        })
    }

    pub fn lookup_symbol(&self, name: &str) -> Result<*const c_void> {
        unsafe {
            self.lib
                .symbol::<*const c_void>(name)
                .map_err(|e| napi::Error::from_reason(format!("Symbol '{name}' not found: {e}")))
        }
    }
}
