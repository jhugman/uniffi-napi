use dlopen2::raw::Library;
use napi::Result;
use std::ffi::c_void;

/// Holds a dlopen handle for a native library.
pub struct LibraryHandle {
    pub lib: Library,
}

// Safety: Library handle is only used from the main thread via napi calls.
unsafe impl Send for LibraryHandle {}

impl LibraryHandle {
    pub fn open(path: &str) -> Result<Self> {
        let lib = Library::open(path)
            .map_err(|e| napi::Error::from_reason(format!("dlopen failed for '{path}': {e}")))?;

        Ok(Self { lib })
    }

    pub fn lookup_symbol(&self, name: &str) -> Result<*const c_void> {
        unsafe {
            self.lib
                .symbol::<*const c_void>(name)
                .map_err(|e| napi::Error::from_reason(format!("Symbol '{name}' not found: {e}")))
        }
    }
}
