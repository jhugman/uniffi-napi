# uniffi-napi Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `uniffi-napi`, a napi-rs native addon that replaces ffi-rs as the FFI bridge for uniffi-bindgen-node, with proper threading, BigInt, and struct-by-value support.

**Architecture:** A new standalone napi-rs project at `/Users/jhugman/workspaces/uniffi-napi`. Uses dlopen to load target libraries at runtime and libffi to call their exported C functions. JS↔C marshaling is specialized for UniFFI's type system. Callbacks dispatch via ThreadsafeFunction with main-thread detection.

**Tech Stack:** Rust (napi-rs, libffi, libffi-sys, dlopen, libc), TypeScript (tests), Node.js N-API v6+

**Spec:** `../specs/2026-03-10-uniffi-napi-design.md`

---

## File Structure

### New project: `/Users/jhugman/workspaces/uniffi-napi/`

```
uniffi-napi/
├── Cargo.toml                  # napi-rs cdylib, deps: napi, libffi, libffi-sys, dlopen, libc
├── build.rs                    # napi_build::setup()
├── package.json                # napi build config, test scripts
├── tsconfig.json               # TypeScript config for tests
├── src/
│   ├── lib.rs                  # #[module_init], main thread ID capture, re-exports
│   ├── ffi_type.rs             # FfiTypeDesc enum, parsing from JS objects
│   ├── cif.rs                  # libffi CIF building from FfiTypeDesc
│   ├── library.rs              # UniffiNativeModule: dlopen, symbol lookup, rustbuffer symbols
│   ├── register.rs             # register(): build CIFs, create callable JS functions
│   ├── marshal.rs              # JS ↔ C marshaling: scalars, RustBuffer, RustCallStatus
│   ├── callback.rs             # Callback trampolines, ThreadsafeFunction, main-thread dispatch
│   └── structs.rs              # C struct building from JS objects (RustBuffer, RustCallStatus, VTables)
├── test_lib/
│   ├── Cargo.toml              # cdylib test fixture
│   └── src/
│       └── lib.rs              # extern "C" functions mimicking UniFFI scaffolding
└── tests/
    ├── open_close.test.mjs     # dlopen/close tests
    ├── scalars.test.mjs        # scalar arg/return tests
    ├── rust_buffer.test.mjs    # RustBuffer marshaling tests
    ├── rust_call_status.test.mjs # RustCallStatus output param tests
    ├── callbacks.test.mjs      # Callback trampoline tests
    └── threading.test.mjs      # Cross-thread callback dispatch tests
```

**Module responsibilities:**
- `index.js` — JS wrapper: re-exports native module + defines `FfiType` constant object
- `lib.rs` — Module init, main thread ID, exports `open()` as `#[napi]`
- `ffi_type.rs` — `FfiTypeDesc` enum matching uniffi's `FfiType`, parsing from tagged JS objects
- `cif.rs` — Converts `FfiTypeDesc` → `libffi_sys::ffi_type`, builds `ffi_cif` for function signatures
- `library.rs` — `UniffiNativeModule` struct: holds dlopen handle, rustbuffer symbols, registered CIFs
- `register.rs` — Takes JS definition object, builds CIFs, returns JS object with callable properties
- `marshal.rs` — Per-type JS↔C conversion: `napi::JsNumber` → `i32`, `JsTypedArray` → `RustBuffer`, etc.
- `callback.rs` — `ffi_closure_alloc` trampolines, `ThreadsafeFunction` wrapping, thread detection
- `structs.rs` — Build C structs (RustBuffer, RustCallStatus, FfiStruct) from JS objects and vice versa

---

## Chunk 1: Project Scaffold + Scalar Function Calls

### Task 1: Project scaffold

**Files:**
- Create: `uniffi-napi/Cargo.toml`
- Create: `uniffi-napi/build.rs`
- Create: `uniffi-napi/package.json`
- Create: `uniffi-napi/tsconfig.json`
- Create: `uniffi-napi/src/lib.rs`

- [ ] **Step 1: Create project directory and Cargo.toml**

```bash
mkdir -p /Users/jhugman/workspaces/uniffi-napi/src
```

Create `Cargo.toml`:
```toml
[package]
name = "uniffi-napi"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
napi = { version = "2.16", default-features = false, features = ["napi6"] }
napi-derive = "2.16"
dlopen2 = "0.7"
libffi = "5.0"
libffi-sys = "4.0"
libc = "0.2"

[build-dependencies]
napi-build = "2.1"
```

- [ ] **Step 2: Create build.rs**

```rust
extern crate napi_build;

fn main() {
    napi_build::setup();
}
```

- [ ] **Step 3: Create package.json**

```json
{
  "name": "uniffi-napi",
  "version": "0.1.0",
  "main": "index.js",
  "types": "index.d.ts",
  "napi": {
    "name": "uniffi-napi",
    "triples": {
      "defaults": true,
      "additional": [
        "aarch64-apple-darwin",
        "aarch64-unknown-linux-gnu"
      ]
    }
  },
  "scripts": {
    "build": "napi build --platform --release",
    "build:debug": "napi build --platform",
    "test": "node --test tests/*.test.mjs"
  },
  "devDependencies": {
    "@napi-rs/cli": "^2.18.0"
  }
}
```

- [ ] **Step 4: Create minimal src/lib.rs with module init**

```rust
#[macro_use]
extern crate napi_derive;

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
```

- [ ] **Step 5: Verify it compiles**

```bash
cd /Users/jhugman/workspaces/uniffi-napi
npm install
npm run build:debug
```

Expected: builds successfully, produces `.node` binary

- [ ] **Step 6: Commit**

```bash
git init
git add .
git commit -m "feat: project scaffold with napi-rs, libffi, dlopen deps"
```

### Task 2: FfiType descriptors

**Files:**
- Create: `uniffi-napi/src/ffi_type.rs`

- [ ] **Step 1: Define FfiTypeDesc enum**

```rust
// src/ffi_type.rs
use napi::bindgen_prelude::*;
use napi::{JsObject, Result};

/// Mirrors uniffi_bindgen's FfiType enum.
/// Parsed from tagged JS objects like { tag: 'Int32' } or { tag: 'Callback', name: 'cb_name' }.
#[derive(Debug, Clone)]
pub enum FfiTypeDesc {
    UInt8,
    Int8,
    UInt16,
    Int16,
    UInt32,
    Int32,
    UInt64,
    Int64,
    Float32,
    Float64,
    Handle,
    RustBuffer,
    ForeignBytes,
    RustCallStatus,
    Callback(String),
    Struct(String),
    Reference(Box<FfiTypeDesc>),
    MutReference(Box<FfiTypeDesc>),
    VoidPointer,
    Void,
}

impl FfiTypeDesc {
    /// Parse from a JS object with { tag: string, ...params }.
    pub fn from_js_object(env: &Env, obj: &JsObject) -> Result<Self> {
        let tag: String = obj.get_named_property::<String>("tag")?;
        match tag.as_str() {
            "UInt8" => Ok(Self::UInt8),
            "Int8" => Ok(Self::Int8),
            "UInt16" => Ok(Self::UInt16),
            "Int16" => Ok(Self::Int16),
            "UInt32" => Ok(Self::UInt32),
            "Int32" => Ok(Self::Int32),
            "UInt64" => Ok(Self::UInt64),
            "Int64" => Ok(Self::Int64),
            "Float32" => Ok(Self::Float32),
            "Float64" => Ok(Self::Float64),
            "Handle" => Ok(Self::Handle),
            "RustBuffer" => Ok(Self::RustBuffer),
            "ForeignBytes" => Ok(Self::ForeignBytes),
            "RustCallStatus" => Ok(Self::RustCallStatus),
            "VoidPointer" => Ok(Self::VoidPointer),
            "Void" => Ok(Self::Void),
            "Callback" => {
                let name: String = obj.get_named_property::<String>("name")?;
                Ok(Self::Callback(name))
            }
            "Struct" => {
                let name: String = obj.get_named_property::<String>("name")?;
                Ok(Self::Struct(name))
            }
            "Reference" => {
                let inner: JsObject = obj.get_named_property("inner")?;
                Ok(Self::Reference(Box::new(Self::from_js_object(env, &inner)?)))
            }
            "MutReference" => {
                let inner: JsObject = obj.get_named_property("inner")?;
                Ok(Self::MutReference(Box::new(Self::from_js_object(env, &inner)?)))
            }
            other => Err(napi::Error::from_reason(format!(
                "Unknown FfiType tag: {other}"
            ))),
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Add `mod ffi_type;` to `lib.rs` (already present from Task 1). Run `npm run build:debug`. Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add src/ffi_type.rs
git commit -m "feat: FfiTypeDesc enum with JS object parsing"
```

### Task 3: Test fixture library

**Files:**
- Create: `uniffi-napi/test_lib/Cargo.toml`
- Create: `uniffi-napi/test_lib/src/lib.rs`

- [ ] **Step 1: Create test_lib Cargo.toml**

```toml
[package]
name = "uniffi-napi-test-lib"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]
```

- [ ] **Step 2: Write test fixture with extern "C" functions**

```rust
// test_lib/src/lib.rs
use std::ptr;

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
    if !buf.data.is_null() && buf.capacity > 0 {
        let layout =
            std::alloc::Layout::from_size_align(buf.capacity as usize, 1).unwrap();
        unsafe { std::alloc::dealloc(buf.data, layout) };
    }
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
    // Return a copy, free the input
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
    // Free input
    if buf.capacity > 0 && !buf.data.is_null() {
        let old_layout =
            std::alloc::Layout::from_size_align(buf.capacity as usize, 1).unwrap();
        unsafe { std::alloc::dealloc(buf.data, old_layout) };
    }
    RustBuffer {
        capacity: len as u64,
        len: len as u64,
        data: new_data,
    }
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
```

- [ ] **Step 3: Build test fixture**

```bash
cd /Users/jhugman/workspaces/uniffi-napi/test_lib
cargo build
```

Expected: produces `target/debug/libuniffi_napi_test_lib.dylib` (or `.so` on Linux)

- [ ] **Step 4: Commit**

```bash
cd /Users/jhugman/workspaces/uniffi-napi
git add test_lib/
git commit -m "feat: test fixture library with UniFFI-style extern C functions"
```

### Task 4: Library open/close

**Files:**
- Create: `uniffi-napi/src/library.rs`
- Modify: `uniffi-napi/src/lib.rs`

- [ ] **Step 1: Write open_close test**

Create `tests/open_close.test.mjs`:
```javascript
import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import { UniffiNativeModule } from '../index.js';

const LIB_PATH = join(import.meta.dirname, '..', 'test_lib', 'target', 'debug',
  process.platform === 'darwin' ? 'libuniffi_napi_test_lib.dylib' : 'libuniffi_napi_test_lib.so'
);

test('open() loads a library', () => {
  const lib = UniffiNativeModule.open(LIB_PATH, {
    rustbufferAlloc: 'uniffi_test_rustbuffer_alloc',
    rustbufferFree: 'uniffi_test_rustbuffer_free',
    rustbufferFromBytes: 'uniffi_test_rustbuffer_from_bytes',
  });
  assert.ok(lib);
  lib.close();
});

test('open() throws for nonexistent library', () => {
  assert.throws(() => {
    UniffiNativeModule.open('/nonexistent/lib.dylib', {
      rustbufferAlloc: 'x',
      rustbufferFree: 'x',
      rustbufferFromBytes: 'x',
    });
  }, /Error/);
});

test('open() throws for missing symbol', () => {
  assert.throws(() => {
    UniffiNativeModule.open(LIB_PATH, {
      rustbufferAlloc: 'nonexistent_symbol',
      rustbufferFree: 'uniffi_test_rustbuffer_free',
      rustbufferFromBytes: 'uniffi_test_rustbuffer_from_bytes',
    });
  }, /Error/);
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test
```

Expected: FAIL — `UniffiNativeModule` not exported

- [ ] **Step 3: Implement library.rs**

```rust
// src/library.rs
use dlopen2::raw::Library;
use napi::bindgen_prelude::*;
use napi::Result;
use std::collections::HashMap;
use std::ffi::c_void;

/// Holds a dlopen handle and pre-resolved rustbuffer management symbols.
pub struct LibraryHandle {
    pub lib: Library,
    pub rustbuffer_alloc: *const c_void,
    pub rustbuffer_free: *const c_void,
    pub rustbuffer_from_bytes: *const c_void,
}

// Safety: symbol pointers are valid for the lifetime of the Library handle.
// They are only called from the main thread or via ThreadsafeFunction dispatch.
unsafe impl Send for LibraryHandle {}

impl LibraryHandle {
    pub fn open(path: &str, alloc: &str, free: &str, from_bytes: &str) -> Result<Self> {
        let lib = Library::open(path)
            .map_err(|e| napi::Error::from_reason(format!("dlopen failed for '{path}': {e}")))?;

        // dlopen2::raw::Library::symbol() returns the raw pointer directly.
        let rustbuffer_alloc = unsafe {
            lib.symbol::<*const c_void>(alloc)
                .map_err(|e| napi::Error::from_reason(format!("Symbol '{alloc}' not found: {e}")))?
        };
        let rustbuffer_free = unsafe {
            lib.symbol::<*const c_void>(free)
                .map_err(|e| napi::Error::from_reason(format!("Symbol '{free}' not found: {e}")))?
        };
        let rustbuffer_from_bytes = unsafe {
            lib.symbol::<*const c_void>(from_bytes)
                .map_err(|e| {
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
                .map_err(|e| {
                    napi::Error::from_reason(format!("Symbol '{name}' not found: {e}"))
                })
        }
    }
}
```

Note: `dlopen2::raw::Library` is used (not `dlopen2::symbor::Library`) because `raw` returns pointers directly without lifetime-bound wrappers. The `LibraryHandle` owns the `Library` and all symbol pointers are valid for its lifetime.

- [ ] **Step 4: Implement UniffiNativeModule in lib.rs**

Update `src/lib.rs`:
```rust
#[macro_use]
extern crate napi_derive;

use std::sync::OnceLock;
use std::thread::ThreadId;

mod ffi_type;
mod library;

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
    pub fn open(path: String) -> napi::Result<Self> {
        let handle = LibraryHandle::open(&path)?;
        Ok(Self {
            handle: Some(handle),
        })
    }

    #[napi]
    pub fn close(&mut self) {
        self.handle.take();
    }
}
```

- [ ] **Step 5: Build and run test**

```bash
npm run build:debug && npm test
```

Expected: all 3 tests pass

- [ ] **Step 6: Commit**

```bash
git add src/library.rs src/lib.rs tests/open_close.test.mjs
git commit -m "feat: UniffiNativeModule.open() and close() with dlopen"
```

### Task 5: Scalar function calls via register()

**Files:**
- Create: `uniffi-napi/src/cif.rs`
- Create: `uniffi-napi/src/marshal.rs`
- Create: `uniffi-napi/src/register.rs`
- Modify: `uniffi-napi/src/lib.rs`
- Create: `uniffi-napi/tests/scalars.test.mjs`

- [ ] **Step 1: Write scalar test**

Create `tests/scalars.test.mjs`:
```javascript
import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import { UniffiNativeModule, FfiType } from '../index.js';

const LIB_PATH = join(import.meta.dirname, '..', 'test_lib', 'target', 'debug',
  process.platform === 'darwin' ? 'libuniffi_napi_test_lib.dylib' : 'libuniffi_napi_test_lib.so'
);

function openLib() {
  return UniffiNativeModule.open(LIB_PATH, {
    rustbufferAlloc: 'uniffi_test_rustbuffer_alloc',
    rustbufferFree: 'uniffi_test_rustbuffer_free',
    rustbufferFromBytes: 'uniffi_test_rustbuffer_from_bytes',
  });
}

test('register and call i32 add function', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {},
    functions: {
      uniffi_test_fn_add: {
        args: [FfiType.Int32, FfiType.Int32],
        ret: FfiType.Int32,
        hasRustCallStatus: true,
      },
    },
  });

  const status = { code: 0 };
  const result = nm.uniffi_test_fn_add(3, 4, status);
  assert.strictEqual(result, 7);
  assert.strictEqual(status.code, 0);
  lib.close();
});

test('register and call i8 negate function', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {},
    functions: {
      uniffi_test_fn_negate: {
        args: [FfiType.Int8],
        ret: FfiType.Int8,
        hasRustCallStatus: true,
      },
    },
  });

  const status = { code: 0 };
  const result = nm.uniffi_test_fn_negate(42, status);
  assert.strictEqual(result, -42);
  lib.close();
});

test('register and call u64 handle function', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {},
    functions: {
      uniffi_test_fn_handle: {
        args: [],
        ret: FfiType.Handle,
        hasRustCallStatus: true,
      },
    },
  });

  const status = { code: 0 };
  const result = nm.uniffi_test_fn_handle(status);
  assert.strictEqual(result, 0xDEADBEEF12345678n);
  lib.close();
});

test('register and call void function', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {},
    functions: {
      uniffi_test_fn_void: {
        args: [],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
    },
  });

  const status = { code: 0 };
  const result = nm.uniffi_test_fn_void(status);
  assert.strictEqual(result, undefined);
  assert.strictEqual(status.code, 0);
  lib.close();
});

test('register and call f64 double function', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {},
    functions: {
      uniffi_test_fn_double: {
        args: [FfiType.Float64],
        ret: FfiType.Float64,
        hasRustCallStatus: true,
      },
    },
  });

  const status = { code: 0 };
  const result = nm.uniffi_test_fn_double(3.14, status);
  assert.strictEqual(result, 6.28);
  lib.close();
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test
```

Expected: FAIL — `FfiType` not exported, `register` not defined

- [ ] **Step 3: Implement cif.rs — libffi CIF building**

This module converts `FfiTypeDesc` into libffi `ffi_type` pointers and builds `ffi_cif` structs. Key implementation notes:

- Scalar types map to `libffi_sys::ffi_type_sint32`, `ffi_type_uint64`, etc.
- `Handle` maps to `ffi_type_uint64`
- `RustCallStatus` as a function arg is always passed as `MutReference(RustCallStatus)`, i.e. a pointer — use `ffi_type_pointer`
- `RustBuffer` by value requires building a custom `ffi_type` with `FFI_TYPE_STRUCT` and the three fields (u64, u64, pointer)
- Store `ffi_cif` + the `ffi_type` pointers together so they don't get dropped

The implementer should consult `libffi` and `libffi-sys` crate docs for the exact API. The `libffi::middle` module provides a safer API than raw `libffi-sys`.

- [ ] **Step 4: Implement marshal.rs — JS ↔ C scalar conversion**

Two key functions per type direction:
- `js_to_c(env, ffi_type, js_value) -> *mut c_void` — reads a napi value, writes to a C-typed allocation
- `c_to_js(env, ffi_type, *const c_void) -> JsUnknown` — reads a C value, creates a napi value

For scalars:
- `Int8/UInt8/Int16/UInt16/Int32/UInt32/Float32/Float64` → `env.get_value_*()` / `env.create_*()` via JsNumber
- `Int64/UInt64/Handle` → `env.get_value_bigint_*()` / `env.create_bigint_*()` via JsBigInt
- `Void` → no marshaling needed

For `RustCallStatus`: read JS object's `code` property, create C struct on stack, pass `&mut` pointer. After call, write back. This is special-cased in the call dispatch, not in generic marshal.

- [ ] **Step 5: Implement register.rs — register() method**

`register()` takes a JS object with `{ structs, callbacks, functions }`, iterates the `functions` map, and for each:

1. Parses the `args` array and `ret` into `Vec<FfiTypeDesc>` and `FfiTypeDesc`
2. Looks up the symbol name via `LibraryHandle::lookup_symbol()`
3. Builds a `ffi_cif` via `cif.rs`
4. Creates a JS function (via `env.create_function_from_closure()`) that:
   - Reads each JS argument using `marshal.rs`
   - If `hasRustCallStatus`, reads the last JS arg as `{ code }`, creates a C `RustCallStatus`
   - Calls `ffi_call` with the CIF, symbol pointer, and marshaled args
   - If `hasRustCallStatus`, writes mutated status back to JS object
   - Marshals the return value back to JS
5. Sets each function as a property on a result `JsObject`

Add `register()` to `UniffiNativeModule`. Note: `register()` may need to store trampolines on the module for VTable callbacks (Chunk 3). Use `&mut self` or wrap internal state in `RefCell`/`Mutex` for interior mutability. For Chunk 1 (scalar-only), `&self` with immutable access is sufficient since CIFs are owned by the returned closures.

```rust
#[napi]
pub fn register(&self, env: Env, definitions: JsObject) -> napi::Result<JsObject> {
    let handle = self.handle.as_ref()
        .ok_or_else(|| napi::Error::from_reason("Module is closed"))?;
    register::register(env, handle, definitions)
}
```

**Critical: RustCallStatus in the CIF.** When `hasRustCallStatus` is true, the CIF must include one extra arg beyond the declared `args`: a `ffi_type_pointer` for the `&mut RustCallStatus`. The JS-visible args are `[...declared_args, statusObject]`, but the C-level args are `[...declared_args, pointer_to_c_struct]`. Concretely:
1. Build CIF arg types = declared `args` mapped to ffi_types, PLUS one `ffi_type_pointer` at the end
2. Before `ffi_call`: allocate a C `RustCallStatus` on the stack, zero it, set `code` from JS object
3. Append `&mut rust_call_status` as the last C arg pointer
4. After `ffi_call`: write back `code` from the C struct to the JS object (error_buf writeback is deferred to Task 7)

Also create `index.js` now (not deferred) so tests can import `FfiType`:

```javascript
// index.js
const native = require('./uniffi-napi.node');
module.exports = {
  ...native,
  FfiType: {
    UInt8: { tag: 'UInt8' },
    Int8: { tag: 'Int8' },
    UInt16: { tag: 'UInt16' },
    Int16: { tag: 'Int16' },
    UInt32: { tag: 'UInt32' },
    Int32: { tag: 'Int32' },
    UInt64: { tag: 'UInt64' },
    Int64: { tag: 'Int64' },
    Float32: { tag: 'Float32' },
    Float64: { tag: 'Float64' },
    Handle: { tag: 'Handle' },
    RustBuffer: { tag: 'RustBuffer' },
    ForeignBytes: { tag: 'ForeignBytes' },
    RustCallStatus: { tag: 'RustCallStatus' },
    VoidPointer: { tag: 'VoidPointer' },
    Void: { tag: 'Void' },
    Callback: (name) => ({ tag: 'Callback', name }),
    Struct: (name) => ({ tag: 'Struct', name }),
    Reference: (inner) => ({ tag: 'Reference', inner }),
    MutReference: (inner) => ({ tag: 'MutReference', inner }),
  },
};
```

- [ ] **Step 6: Build and run tests**

```bash
npm run build:debug && npm test
```

Expected: all scalar tests pass (add, negate, handle, void, double)

- [ ] **Step 7: Commit**

```bash
git add src/cif.rs src/marshal.rs src/register.rs src/lib.rs tests/scalars.test.mjs
git commit -m "feat: register() with scalar function calls via libffi"
```

---

## Chunk 2: RustBuffer + RustCallStatus Marshaling

### Task 6: RustBuffer marshaling

**Files:**
- Modify: `uniffi-napi/src/marshal.rs`
- Modify: `uniffi-napi/src/cif.rs` (add RustBuffer ffi_type)
- Create: `uniffi-napi/tests/rust_buffer.test.mjs`

- [ ] **Step 1: Write RustBuffer test**

Create `tests/rust_buffer.test.mjs`:
```javascript
import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import { UniffiNativeModule, FfiType } from '../index.js';

const LIB_PATH = join(import.meta.dirname, '..', 'test_lib', 'target', 'debug',
  process.platform === 'darwin' ? 'libuniffi_napi_test_lib.dylib' : 'libuniffi_napi_test_lib.so'
);

function openLib() {
  return UniffiNativeModule.open(LIB_PATH, {
    rustbufferAlloc: 'uniffi_test_rustbuffer_alloc',
    rustbufferFree: 'uniffi_test_rustbuffer_free',
    rustbufferFromBytes: 'uniffi_test_rustbuffer_from_bytes',
  });
}

test('RustBuffer echo: pass Uint8Array, get same bytes back', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {},
    functions: {
      uniffi_test_fn_echo_buffer: {
        args: [FfiType.RustBuffer],
        ret: FfiType.RustBuffer,
        hasRustCallStatus: true,
      },
    },
  });

  const input = new Uint8Array([1, 2, 3, 4, 5]);
  const status = { code: 0 };
  const result = nm.uniffi_test_fn_echo_buffer(input, status);

  assert.strictEqual(status.code, 0);
  assert.ok(result instanceof Uint8Array);
  assert.deepStrictEqual(result, input);
  lib.close();
});

test('RustBuffer echo: empty buffer', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {},
    functions: {
      uniffi_test_fn_echo_buffer: {
        args: [FfiType.RustBuffer],
        ret: FfiType.RustBuffer,
        hasRustCallStatus: true,
      },
    },
  });

  const input = new Uint8Array([]);
  const status = { code: 0 };
  const result = nm.uniffi_test_fn_echo_buffer(input, status);

  assert.strictEqual(status.code, 0);
  assert.ok(result instanceof Uint8Array);
  assert.strictEqual(result.length, 0);
  lib.close();
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test
```

Expected: FAIL — RustBuffer type not handled in marshal/cif

- [ ] **Step 3: Add RustBuffer ffi_type to cif.rs**

Build a custom `ffi_type` struct for `RustBuffer`:
- Field 0: `ffi_type_uint64` (capacity)
- Field 1: `ffi_type_uint64` (len)
- Field 2: `ffi_type_pointer` (data)
- `type_` = `FFI_TYPE_STRUCT`

The struct definition must be heap-allocated and kept alive for the CIF's lifetime.

- [ ] **Step 4: Add RustBuffer lowering to marshal.rs**

**Calling rustbuffer management symbols:** The three symbols (`rustbuffer_alloc`, `rustbuffer_from_bytes`, `rustbuffer_free`) are stored as `*const c_void` in `LibraryHandle`. Since their signatures are known at compile time, the simplest approach is to `transmute` them to typed `extern "C" fn(...)` pointers:
```rust
let from_bytes: extern "C" fn(ForeignBytes, &mut RustCallStatus) -> RustBuffer =
    unsafe { std::mem::transmute(handle.rustbuffer_from_bytes) };
let free: extern "C" fn(RustBuffer, &mut RustCallStatus) =
    unsafe { std::mem::transmute(handle.rustbuffer_free) };
```

Note: all three management functions take `&mut RustCallStatus` as their last parameter. Allocate a local `RustCallStatus` for each management call and check it.

When lowering a `Uint8Array` arg tagged as `RustBuffer`:
1. Get the `Uint8Array` data pointer and length from napi
2. Hold a `napi::Ref` to prevent GC during the `rustbuffer_from_bytes` call (which copies the bytes; the ref can be released after that call returns)
3. Create a `ForeignBytes { len: data.len() as i32, data: data.as_ptr() }`
4. Call the library's `rustbuffer_from_bytes` symbol with the ForeignBytes and a `&mut RustCallStatus`
5. Get back a C `RustBuffer` struct
6. Pass that struct by value to the main `ffi_call`

When lifting a `RustBuffer` return value:
1. Read `len` and `data` from the returned struct
2. Create a `Uint8Array` in napi, copy `len` bytes from `data`
3. Call the library's `rustbuffer_free` with the struct and a `&mut RustCallStatus`
4. Return the `Uint8Array`

- [ ] **Step 5: Build and run tests**

```bash
npm run build:debug && npm test
```

Expected: RustBuffer echo tests pass

- [ ] **Step 6: Commit**

```bash
git add src/marshal.rs src/cif.rs tests/rust_buffer.test.mjs
git commit -m "feat: RustBuffer marshaling (Uint8Array <-> C struct by value)"
```

### Task 7: RustCallStatus error reporting

**Files:**
- Modify: `uniffi-napi/src/marshal.rs` (or `register.rs` call dispatch)
- Create: `uniffi-napi/tests/rust_call_status.test.mjs`

- [ ] **Step 1: Write RustCallStatus error test**

Create `tests/rust_call_status.test.mjs`:
```javascript
import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import { UniffiNativeModule, FfiType } from '../index.js';

const LIB_PATH = join(import.meta.dirname, '..', 'test_lib', 'target', 'debug',
  process.platform === 'darwin' ? 'libuniffi_napi_test_lib.dylib' : 'libuniffi_napi_test_lib.so'
);

function openLib() {
  return UniffiNativeModule.open(LIB_PATH, {
    rustbufferAlloc: 'uniffi_test_rustbuffer_alloc',
    rustbufferFree: 'uniffi_test_rustbuffer_free',
    rustbufferFromBytes: 'uniffi_test_rustbuffer_from_bytes',
  });
}

test('RustCallStatus: error code and errorBuf are written back', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {},
    functions: {
      uniffi_test_fn_error: {
        args: [],
        ret: FfiType.Int32,
        hasRustCallStatus: true,
      },
    },
  });

  const status = { code: 0 };
  nm.uniffi_test_fn_error(status);

  assert.strictEqual(status.code, 2); // CALL_UNEXPECTED_ERROR
  assert.ok(status.errorBuf instanceof Uint8Array);
  assert.strictEqual(status.errorBuf.length, 20); // "something went wrong"
  const msg = new TextDecoder().decode(status.errorBuf);
  assert.strictEqual(msg, 'something went wrong');
  lib.close();
});

test('RustCallStatus: success has no errorBuf', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {},
    functions: {
      uniffi_test_fn_add: {
        args: [FfiType.Int32, FfiType.Int32],
        ret: FfiType.Int32,
        hasRustCallStatus: true,
      },
    },
  });

  const status = { code: 0 };
  nm.uniffi_test_fn_add(1, 2, status);

  assert.strictEqual(status.code, 0);
  assert.strictEqual(status.errorBuf, undefined);
  lib.close();
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test
```

Expected: FAIL — error_buf not being written back to JS object

- [ ] **Step 3: Implement RustCallStatus writeback**

In the call dispatch (register.rs), after `ffi_call`:
1. Read `code` from the C `RustCallStatus` struct
2. Set `status_obj.code = code` on the JS object
3. If `code != 0` and `error_buf.data` is non-null:
   - Copy `error_buf.len` bytes into a new `Uint8Array`
   - Set `status_obj.errorBuf = uint8array`
   - Call `rustbuffer_free` on the error_buf

- [ ] **Step 4: Build and run tests**

```bash
npm run build:debug && npm test
```

Expected: all RustCallStatus tests pass

- [ ] **Step 5: Commit**

```bash
git add src/marshal.rs src/register.rs tests/rust_call_status.test.mjs
git commit -m "feat: RustCallStatus error_buf writeback to JS object"
```

---

## Chunk 3: Callbacks + Threading

### Task 8: Same-thread callback trampolines

**Files:**
- Create: `uniffi-napi/src/callback.rs`
- Modify: `uniffi-napi/src/register.rs`
- Modify: `uniffi-napi/src/cif.rs`
- Create: `uniffi-napi/tests/callbacks.test.mjs`

- [ ] **Step 1: Write same-thread callback test**

Create `tests/callbacks.test.mjs`:
```javascript
import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import { UniffiNativeModule, FfiType } from '../index.js';

const LIB_PATH = join(import.meta.dirname, '..', 'test_lib', 'target', 'debug',
  process.platform === 'darwin' ? 'libuniffi_napi_test_lib.dylib' : 'libuniffi_napi_test_lib.so'
);

function openLib() {
  return UniffiNativeModule.open(LIB_PATH, {
    rustbufferAlloc: 'uniffi_test_rustbuffer_alloc',
    rustbufferFree: 'uniffi_test_rustbuffer_free',
    rustbufferFromBytes: 'uniffi_test_rustbuffer_from_bytes',
  });
}

test('callback: same-thread invocation', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {
      simple_callback: {
        args: [FfiType.UInt64, FfiType.Int8],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
    },
    functions: {
      uniffi_test_fn_call_callback: {
        args: [FfiType.Callback('simple_callback'), FfiType.UInt64, FfiType.Int8],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
    },
  });

  let receivedHandle = null;
  let receivedValue = null;
  const callback = (handle, value) => {
    receivedHandle = handle;
    receivedValue = value;
  };

  const status = { code: 0 };
  nm.uniffi_test_fn_call_callback(callback, 42n, 7, status);

  assert.strictEqual(status.code, 0);
  assert.strictEqual(receivedHandle, 42n);
  assert.strictEqual(receivedValue, 7);
  lib.close();
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test
```

Expected: FAIL — Callback type not handled

- [ ] **Step 3: Implement callback.rs**

Key implementation:
1. Use `libffi::middle::Closure` (or raw `ffi_closure_alloc`) to create a C function pointer
2. The closure captures a reference to the JS function (via `napi::Ref` or `JsFunction`)
3. Since this is same-thread (main thread check), the trampoline directly calls the JS function via the captured `Env`
4. For `Callback(name)` args in a function, look up the callback definition to know the arg types
5. In the trampoline: marshal C args → JS values, call JS function, marshal return

**RustCallStatus in callback trampolines:** When a callback has `hasRustCallStatus: true`, the C-level signature includes an extra `&mut RustCallStatus` parameter beyond the declared `args`. The trampoline CIF must include `ffi_type_pointer` for this parameter. When the trampoline fires:
1. Read `code` from the C `RustCallStatus` into a JS `{ code: N }` object
2. Pass the JS object as the last arg to the JS function
3. After the JS function returns, write back `code` from the JS object to the C struct
4. If the JS function set `errorBuf` on the status object, marshal the `Uint8Array` into a C `RustBuffer` and write it to the C struct's `error_buf` field

**Blocking vs NonBlocking dispatch:** For callbacks with non-void return types (like VTable methods), cross-thread dispatch must use `ThreadsafeFunctionCallMode::Blocking` so the Rust thread waits for the JS result. For void callbacks (like async continuations), use `NonBlocking`. The choice is determined by the callback's `ret` type at trampoline creation time.

Critical: `ffi_closure_alloc` returns executable memory. The closure must be kept alive as long as the C function pointer could be called. For function-argument callbacks (not VTable), the lifetime is the duration of the `ffi_call`. For callbacks passed to async APIs (like rust_future_poll continuations), store the trampoline in the `UniffiNativeModule` and free on `close()`, since there's no signal for when the callback is no longer needed.

- [ ] **Step 4: Wire callbacks into register.rs**

When processing a function arg of type `Callback(name)`:
1. Read the JS function from the positional argument
2. Look up the callback signature from the `callbacks` map (parsed at registration time)
3. Create a trampoline via `callback.rs`
4. Pass the C function pointer to `ffi_call`
5. After `ffi_call` returns, clean up the trampoline (for function-arg callbacks)

- [ ] **Step 5: Build and run tests**

```bash
npm run build:debug && npm test
```

Expected: same-thread callback test passes

- [ ] **Step 6: Commit**

```bash
git add src/callback.rs src/register.rs src/cif.rs tests/callbacks.test.mjs
git commit -m "feat: same-thread callback trampolines via ffi_closure_alloc"
```

### Task 9: Cross-thread callback dispatch

**Files:**
- Modify: `uniffi-napi/src/callback.rs`
- Create: `uniffi-napi/tests/threading.test.mjs`

- [ ] **Step 1: Write cross-thread callback test**

Create `tests/threading.test.mjs`:
```javascript
import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import { UniffiNativeModule, FfiType } from '../index.js';

const LIB_PATH = join(import.meta.dirname, '..', 'test_lib', 'target', 'debug',
  process.platform === 'darwin' ? 'libuniffi_napi_test_lib.dylib' : 'libuniffi_napi_test_lib.so'
);

function openLib() {
  return UniffiNativeModule.open(LIB_PATH, {
    rustbufferAlloc: 'uniffi_test_rustbuffer_alloc',
    rustbufferFree: 'uniffi_test_rustbuffer_free',
    rustbufferFromBytes: 'uniffi_test_rustbuffer_from_bytes',
  });
}

test('callback: invoked from another thread dispatches to event loop', async () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {},
    callbacks: {
      simple_callback: {
        args: [FfiType.UInt64, FfiType.Int8],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
    },
    functions: {
      uniffi_test_fn_call_callback_from_thread: {
        args: [FfiType.Callback('simple_callback'), FfiType.UInt64, FfiType.Int8],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
    },
  });

  const result = await new Promise((resolve) => {
    const callback = (handle, value) => {
      resolve({ handle, value });
    };

    const status = { code: 0 };
    nm.uniffi_test_fn_call_callback_from_thread(callback, 99n, -3, status);
    assert.strictEqual(status.code, 0);
  });

  assert.strictEqual(result.handle, 99n);
  assert.strictEqual(result.value, -3);
  lib.close();
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npm test
```

Expected: FAIL — callback fires on wrong thread, crashes or hangs

- [ ] **Step 3: Add ThreadsafeFunction dispatch to callback.rs**

Modify the callback trampoline to check `is_main_thread()`:

- **If main thread:** call JS function directly (existing path from Task 8)
- **If not main thread:** dispatch via `napi::threadsafe_function::ThreadsafeFunction`
  - Create the TSF from the JS function at trampoline creation time
  - In the trampoline: call `tsfn.call(args, ThreadsafeFunctionCallMode::NonBlocking)`
  - For this non-blocking case (async continuation pattern), the trampoline returns immediately

Key napi-rs API:
```rust
use napi::threadsafe_function::{
    ThreadsafeFunction, ThreadsafeFunctionCallMode, ErrorStrategy,
};
```

The TSF must be created on the main thread (when the callback trampoline is set up), and can then be called from any thread.

For function-argument callbacks that may be called from another thread: the trampoline and TSF must live long enough. Since `uniffi_test_fn_call_callback_from_thread` spawns a thread and returns immediately, the trampoline must outlive the `ffi_call`. Store it in the `UniffiNativeModule` or use `Arc` reference counting. The test fixture's thread sleeps 10ms, so the trampoline needs to survive at least that long.

- [ ] **Step 4: Build and run tests**

```bash
npm run build:debug && npm test
```

Expected: cross-thread callback test passes (callback fires asynchronously on event loop)

- [ ] **Step 5: Commit**

```bash
git add src/callback.rs tests/threading.test.mjs
git commit -m "feat: cross-thread callback dispatch via ThreadsafeFunction"
```

### Task 10: VTable struct passing (for foreign traits)

**Files:**
- Create: `uniffi-napi/src/structs.rs`
- Modify: `uniffi-napi/src/lib.rs` (add `mod structs;`)
- Modify: `uniffi-napi/src/register.rs`
- Modify: `uniffi-napi/test_lib/src/lib.rs` (add VTable test)
- Modify: `uniffi-napi/tests/callbacks.test.mjs` (add VTable test)

- [ ] **Step 1: Add VTable test fixture to test_lib**

Add to `test_lib/src/lib.rs`:
```rust
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
```

- [ ] **Step 2: Write VTable test**

Add to `tests/callbacks.test.mjs`:
```javascript
test('VTable: register struct with callbacks, call through', () => {
  const lib = openLib();
  const nm = lib.register({
    structs: {
      TestVTable: [
        { name: 'get_value', type: FfiType.Callback('vtable_get_value') },
        { name: 'free', type: FfiType.Callback('vtable_free') },
      ],
    },
    callbacks: {
      vtable_get_value: {
        args: [FfiType.UInt64],
        ret: FfiType.Int32,
        hasRustCallStatus: true,
      },
      vtable_free: {
        args: [FfiType.UInt64],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
    },
    functions: {
      uniffi_test_fn_init_vtable: {
        args: [FfiType.Reference(FfiType.Struct('TestVTable'))],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
      uniffi_test_fn_use_vtable: {
        args: [FfiType.UInt64],
        ret: FfiType.Int32,
        hasRustCallStatus: true,
      },
    },
  });

  // Register VTable with JS callbacks
  const status1 = { code: 0 };
  nm.uniffi_test_fn_init_vtable({
    get_value: (handle, callStatus) => {
      callStatus.code = 0;
      return Number(handle) * 10;
    },
    free: (handle, callStatus) => {
      callStatus.code = 0;
    },
  }, status1);
  assert.strictEqual(status1.code, 0);

  // Call through the VTable
  const status2 = { code: 0 };
  const result = nm.uniffi_test_fn_use_vtable(7n, status2);
  assert.strictEqual(result, 70);
  lib.close();
});
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cd /Users/jhugman/workspaces/uniffi-napi/test_lib && cargo build
cd /Users/jhugman/workspaces/uniffi-napi && npm run build:debug && npm test
```

Expected: FAIL — Struct type not handled

- [ ] **Step 4: Implement structs.rs**

When `register.rs` encounters a function arg of type `Reference(Struct(name))` or `Struct(name)`:
1. Delegate to `structs.rs` which looks up the struct definition from the `structs` map
2. Accept a JS object with properties matching field names
3. For each field:
   - If `Callback(name)`: wrap the JS function in a trampoline (reuse callback.rs). These trampolines are **long-lived** — store them in the `UniffiNativeModule`.
   - Otherwise: marshal the value using marshal.rs
4. Build the C struct in memory with correct layout (matching `repr(C)`)
5. Return a pointer to the heap-allocated struct to `register.rs`, which passes it to `ffi_call`
6. For `Reference(Struct(...))`, pass the pointer directly. For `Struct(...)` by value, pass the struct contents.

- [ ] **Step 5: Build and run tests**

```bash
npm run build:debug && npm test
```

Expected: VTable test passes — JS callbacks invoked through Rust

- [ ] **Step 6: Commit**

```bash
git add src/structs.rs src/lib.rs src/register.rs test_lib/src/lib.rs tests/callbacks.test.mjs
git commit -m "feat: VTable struct passing with callback trampolines for foreign traits"
```

### Task 11: FfiType export and cleanup

**Files:**
- Modify: `uniffi-napi/src/lib.rs`

- [ ] **Step 1: Export FfiType constants from the module**

Ensure `FfiType` is exported from the napi module so tests can `import { FfiType } from '../index.js'`. This could be:
- A `#[napi(object)]` with static properties
- Or a JS file that re-exports constants alongside the native module

The simplest approach: create an `index.js` wrapper that re-exports the native module plus a hand-written `FfiType` object:

```javascript
// index.js
const native = require('./uniffi-napi.node');
module.exports = {
  ...native,
  FfiType: {
    UInt8: { tag: 'UInt8' },
    Int8: { tag: 'Int8' },
    UInt16: { tag: 'UInt16' },
    Int16: { tag: 'Int16' },
    UInt32: { tag: 'UInt32' },
    Int32: { tag: 'Int32' },
    UInt64: { tag: 'UInt64' },
    Int64: { tag: 'Int64' },
    Float32: { tag: 'Float32' },
    Float64: { tag: 'Float64' },
    Handle: { tag: 'Handle' },
    RustBuffer: { tag: 'RustBuffer' },
    ForeignBytes: { tag: 'ForeignBytes' },
    RustCallStatus: { tag: 'RustCallStatus' },
    VoidPointer: { tag: 'VoidPointer' },
    Void: { tag: 'Void' },
    Callback: (name) => ({ tag: 'Callback', name }),
    Struct: (name) => ({ tag: 'Struct', name }),
    Reference: (inner) => ({ tag: 'Reference', inner }),
    MutReference: (inner) => ({ tag: 'MutReference', inner }),
  },
};
```

Note: `index.js` was already created in Task 5. This task verifies the FfiType constants are complete and correct. If `index.js` is already up to date, skip to Step 2.

- [ ] **Step 2: Run full test suite**

```bash
npm run build:debug && npm test
```

Expected: all tests pass

- [ ] **Step 3: Commit**

```bash
git add index.js
git commit -m "chore: verify FfiType constants are complete in index.js"
```

---

## Summary

| Chunk | Tasks | What it delivers |
|-------|-------|-----------------|
| 1 | Tasks 1-5 | Project scaffold, FfiType parsing, dlopen, scalar function calls |
| 2 | Tasks 6-7 | RustBuffer Uint8Array marshaling, RustCallStatus error writeback |
| 3 | Tasks 8-11 | Callback trampolines, cross-thread ThreadsafeFunction dispatch, VTable structs |

After all chunks: uniffi-napi supports scalar calls, RustBuffer, RustCallStatus, callbacks (same-thread and cross-thread), and VTable structs. This is sufficient to begin the **second plan** — updating uniffi-bindgen-node's templates to use uniffi-napi instead of ffi-rs.
