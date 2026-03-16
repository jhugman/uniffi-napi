# UniFFI Fixture: End-to-End Validation of uniffi-napi

**Date:** 2026-03-16
**Status:** Proposed

## Problem

The existing `test_lib/` is hand-written `extern "C"` — it validates that uniffi-napi can call C functions, but it bypasses UniFFI entirely. We have no proof that uniffi-napi works with real UniFFI-generated scaffolding. This spec designs a minimal UniFFI fixture that exercises every important code path through the actual UniFFI proc macro machinery.

## Goals

Validate that uniffi-napi correctly handles:

1. **Sync calls** (Rust ← JS) with primitives, strings, and errors
2. **Async calls** (Rust ← JS) via the continuation callback polling pattern
3. **Sync callbacks** (JS → Rust) via foreign trait VTables
4. **Async callbacks** (JS → Rust) via foreign futures
5. **RustBuffer** in argument, return, and error positions
6. **Primitives** in argument and return positions

## Non-Goals

- Code generation — we hand-write the JS `register()` definitions to match UniFFI's symbol names
- Exhaustive type coverage — String and u32 are sufficient to prove RustBuffer and scalar paths work
- Object/Handle lifecycle — not tested in this fixture (already covered by test_lib)

---

## Architecture

Three new components:

```
fixtures/uniffi-fixture-simple/     ← UniFFI crate (proc macros)
lib/async.js                        ← Async runtime (polling loop, handle map)
lib/converters.js                   ← String lift/lower helpers
tests/fixture.test.mjs              ← Integration tests
```

The fixture crate is a real UniFFI library. The JS side manually writes `register()` definitions that match UniFFI's generated scaffolding symbols — exactly what a code generator would emit.

**`lib/async.js` and `lib/converters.js` are reusable runtime code**, not test helpers. They will live alongside `lib.js` and be imported by any generated binding. A code generator would `import { uniffiRustCallAsync } from 'uniffi-napi/lib/async.js'` and `import { liftString, lowerString } from 'uniffi-napi/lib/converters.js'`.

**Crate naming:** The Cargo.toml uses `name = "uniffi-fixture-simple"` (hyphens). Cargo converts hyphens to underscores for the library name, producing `libuniffi_fixture_simple.dylib`. UniFFI uses the underscored form in all generated symbol names.

---

## Component 1: Fixture Crate

**Path:** `fixtures/uniffi-fixture-simple/`

### Cargo.toml

```toml
[package]
name = "uniffi-fixture-simple"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
uniffi = { version = "0.31", features = ["cli"] }
thiserror = "2"
```

**Note on async runtime:** UniFFI's scaffolding manages its own Tokio runtime for polling async functions — we do not need to depend on Tokio directly. If compilation reveals a runtime requirement, we will add it. The `async-trait` crate may also be unnecessary if UniFFI 0.31 handles async traits natively; this will be verified empirically.

### src/lib.rs

```rust
uniffi::setup_scaffolding!();

// ---------- Error type ----------
// Exercises RustBuffer in error position: the error message is serialized
// into the RustCallStatus error_buf as a UniFFI-encoded enum.

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ArithmeticError {
    #[error("{reason}")]
    DivisionByZero { reason: String },
}

// ---------- Sync functions (Rust ← JS) ----------

/// Scalar round-trip: u32 args → u32 return.
#[uniffi::export]
pub fn add(a: u32, b: u32) -> u32 {
    a + b
}

/// String round-trip: String arg → String return.
/// Exercises RustBuffer in both argument and return positions.
#[uniffi::export]
pub fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

/// Error path: returns Result with a serialized error enum.
/// Exercises RustBuffer in the error position (RustCallStatus.error_buf).
#[uniffi::export]
pub fn divide(a: f64, b: f64) -> Result<f64, ArithmeticError> {
    if b == 0.0 {
        Err(ArithmeticError::DivisionByZero {
            reason: "cannot divide by zero".to_string(),
        })
    } else {
        Ok(a / b)
    }
}

// ---------- Async functions (Rust ← JS) ----------

/// Async scalar round-trip.
#[uniffi::export]
pub async fn async_add(a: u32, b: u32) -> u32 {
    a + b
}

/// Async string round-trip.
#[uniffi::export]
pub async fn async_greet(name: String) -> String {
    format!("Hello, {name}!")
}

// ---------- Foreign trait: sync callbacks (JS → Rust) ----------

/// A trait implemented in JS and called from Rust.
/// Exercises VTable registration and cross-language method dispatch.
#[uniffi::export(callback_interface)]
pub trait Calculator {
    /// Scalar callback: u32 args → u32 return.
    fn add(&self, a: u32, b: u32) -> u32;

    /// String callback: String args → String return.
    /// Exercises RustBuffer through the callback path.
    fn concatenate(&self, a: String, b: String) -> String;
}

#[uniffi::export]
pub fn use_calculator(calc: Box<dyn Calculator>, a: u32, b: u32) -> u32 {
    calc.add(a, b)
}

#[uniffi::export]
pub fn use_calculator_strings(
    calc: Box<dyn Calculator>,
    a: String,
    b: String,
) -> String {
    calc.concatenate(a, b)
}

/// Cross-thread callback: spawns a std::thread and calls calc.add() from it.
/// Exercises the blocking cross-thread VTable dispatch path — the worker
/// thread blocks until the JS main thread produces the answer via TSF.
#[uniffi::export]
pub fn use_calculator_from_thread(calc: Box<dyn Calculator>, a: u32, b: u32) -> u32 {
    std::thread::spawn(move || calc.add(a, b)).join().unwrap()
}

// ---------- Foreign trait: async callbacks (JS → Rust) ----------

/// An async trait implemented in JS. When Rust calls fetch(),
/// it receives a foreign future handle and polls it to completion.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait AsyncFetcher: Send + Sync {
    async fn fetch(&self, input: String) -> String;
}

#[uniffi::export]
pub async fn use_async_fetcher(
    fetcher: std::sync::Arc<dyn AsyncFetcher>,
    input: String,
) -> String {
    fetcher.fetch(input).await
}
```

### UniFFI Symbol Naming

UniFFI generates scaffolding symbols with predictable names. For crate `uniffi_fixture_simple`:

| Rust declaration | Generated C symbol |
|---|---|
| `fn add(a: u32, b: u32) -> u32` | `uniffi_uniffi_fixture_simple_fn_func_add` |
| `fn greet(name: String) -> String` | `uniffi_uniffi_fixture_simple_fn_func_greet` |
| `fn divide(a: f64, b: f64) -> Result<...>` | `uniffi_uniffi_fixture_simple_fn_func_divide` |
| `async fn async_add(...)` | `uniffi_uniffi_fixture_simple_fn_func_async_add` |
| `async fn async_greet(...)` | `uniffi_uniffi_fixture_simple_fn_func_async_greet` |
| `fn use_calculator(...)` | `uniffi_uniffi_fixture_simple_fn_func_use_calculator` |
| `fn use_calculator_strings(...)` | `uniffi_uniffi_fixture_simple_fn_func_use_calculator_strings` |
| `fn use_calculator_from_thread(...)` | `uniffi_uniffi_fixture_simple_fn_func_use_calculator_from_thread` |
| `fn use_async_fetcher(...)` | `uniffi_uniffi_fixture_simple_fn_func_use_async_fetcher` |
| RustBuffer alloc | `uniffi_uniffi_fixture_simple_rustbuffer_alloc` |
| RustBuffer free | `uniffi_uniffi_fixture_simple_rustbuffer_free` |
| RustBuffer from_bytes | `uniffi_uniffi_fixture_simple_rustbuffer_from_bytes` |

For async functions, UniFFI also generates per-return-type polling/complete/free symbols:

| Purpose | Symbol pattern |
|---|---|
| Poll future | `uniffi_uniffi_fixture_simple_rust_future_poll_u32` (or `_rust_buffer`) |
| Complete future | `uniffi_uniffi_fixture_simple_rust_future_complete_u32` (or `_rust_buffer`) |
| Free future | `uniffi_uniffi_fixture_simple_rust_future_free_u32` (or `_rust_buffer`) |
| Cancel future | `uniffi_uniffi_fixture_simple_rust_future_cancel_u32` (or `_rust_buffer`) |
| Continuation callback | `uniffi_uniffi_fixture_simple_rust_future_continuation_callback_set` |

**Note:** The exact symbol names will be verified by running `nm -gU` on the compiled dylib after the first build. The names above follow UniFFI 0.31 conventions but may need adjustment.

For callback interfaces, UniFFI generates a VTable init function and a VTable struct:

| Purpose | Symbol |
|---|---|
| Init Calculator VTable | `uniffi_uniffi_fixture_simple_fn_init_callback_vtable_calculator` |
| Init AsyncFetcher VTable | `uniffi_uniffi_fixture_simple_fn_init_callback_vtable_asyncfetcher` |

---

## Component 2: Async Runtime (`lib/async.js`)

A small module providing the polling loop for async Rust futures. Adapted from the React Native `async-rust-call.ts`.

### Handle Map

```js
// Maps bigint handles → Promise resolve functions.
// Each in-flight poll gets a unique handle; the continuation callback
// looks up the resolver by handle and calls it with the poll result.
class HandleMap {
  #nextHandle = 1n;
  #map = new Map();

  insert(resolver) {
    const handle = this.#nextHandle;
    this.#nextHandle += 1n;
    this.#map.set(handle, resolver);
    return handle;
  }

  remove(handle) {
    const resolver = this.#map.get(handle);
    this.#map.delete(handle);
    return resolver;
  }
}
```

### Continuation Callback

The continuation callback is a plain JS function registered as a `Callback` with uniffi-napi:

```js
// Signature: (handle: UInt64, pollResult: Int8) -> Void
// hasRustCallStatus: false
// Registered once in the register() definitions.
const resolverMap = new HandleMap();

function continuationCallback(handle, pollResult) {
  const resolve = resolverMap.remove(handle);
  if (resolve) resolve(pollResult);
}
```

This callback is fire-and-forget (void return, no RustCallStatus), so uniffi-napi's existing NonBlocking TSF dispatch handles it correctly from any thread.

### Polling Loop

```js
const POLL_READY = 0;

async function uniffiRustCallAsync(nm, {
  rustFutureFunc,   // () => bigint — initiates the async call
  pollFunc,         // string — symbol name for poll
  completeFunc,     // string — symbol name for complete
  freeFunc,         // string — symbol name for free
  liftFunc,         // (rawResult) => T — converts result to JS
  callStatus,       // { code: 0 } — RustCallStatus object
}) {
  const futureHandle = rustFutureFunc();

  try {
    let pollResult;
    do {
      pollResult = await new Promise(resolve => {
        const handle = resolverMap.insert(resolve);
        nm[pollFunc](futureHandle, continuationCallback, handle);
      });
    } while (pollResult !== POLL_READY);

    const result = nm[completeFunc](futureHandle, callStatus);
    // Check callStatus.code here; throw if non-zero
    return liftFunc ? liftFunc(result) : result;
  } finally {
    nm[freeFunc](futureHandle);
  }
}
```

### Exports

```js
export { HandleMap, continuationCallback, uniffiRustCallAsync, POLL_READY };
```

---

## Component 3: Converters (`lib/converters.js`)

Minimal serialization helpers for UniFFI's wire format. Only String for now.

### UniFFI String Wire Format

Strings are serialized as: **4-byte big-endian Int32 length prefix + UTF-8 bytes**.

```js
const encoder = new TextEncoder();
const decoder = new TextDecoder();

// JS string → Uint8Array (for RustBuffer argument)
function lowerString(s) {
  const encoded = encoder.encode(s);
  const buf = new Uint8Array(4 + encoded.length);
  new DataView(buf.buffer).setInt32(0, encoded.length, false);
  buf.set(encoded, 4);
  return buf;
}

// Uint8Array (from RustBuffer return) → JS string
function liftString(buf) {
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const len = view.getInt32(0, false);
  return decoder.decode(buf.slice(4, 4 + len));
}
```

### UniFFI Error Wire Format

UniFFI errors are serialized as a variant index + fields. For `ArithmeticError::DivisionByZero { reason }`:

```
[4-byte variant index (Int32, big-endian)] [serialized fields...]
```

Where variant index 1 = `DivisionByZero`, and the field is a serialized String.

```js
function liftArithmeticError(buf) {
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const variantIndex = view.getInt32(0, false);
  const reason = liftString(buf.slice(4));
  return { variant: variantIndex, reason };
}
```

---

## Component 4: Tests (`tests/fixture.test.mjs`)

### Test Matrix

| # | Test | Direction | Sync/Async | Types |
|---|------|-----------|------------|-------|
| 1 | `add(3, 4) → 7` | Rust ← JS | Sync | u32 arg+return |
| 2 | `greet("World") → "Hello, World!"` | Rust ← JS | Sync | String (RustBuffer) arg+return |
| 3 | `divide(1.0, 0.0) → error` | Rust ← JS | Sync | f64 args, error with String |
| 4 | `divide(10.0, 2.0) → 5.0` | Rust ← JS | Sync | f64 args, f64 return (success path) |
| 5 | `async_add(3, 4) → 7` | Rust ← JS | Async | u32 via polling loop |
| 6 | `async_greet("World") → "Hello, World!"` | Rust ← JS | Async | String via polling loop |
| 7 | Calculator.add(3, 4) → 7 | JS → Rust | Sync callback | u32 through VTable |
| 8 | Calculator.concatenate("a", "b") → "ab" | JS → Rust | Sync callback | String through VTable |
| 9 | Calculator.add from thread → 7 | JS → Rust | Sync callback (cross-thread) | Blocking VTable dispatch |
| 10 | AsyncFetcher.fetch("input") → result | JS → Rust | Async callback | String via foreign future |

### Register Definitions

The test file will hand-write the full `register()` call with all symbols, callbacks, structs, and functions matching UniFFI's generated scaffolding. This is the exact code a code generator would emit.

```js
const CRATE = 'uniffi_fixture_simple';
const SYMBOLS = {
  rustbufferAlloc: `uniffi_${CRATE}_rustbuffer_alloc`,
  rustbufferFree: `uniffi_${CRATE}_rustbuffer_free`,
  rustbufferFromBytes: `uniffi_${CRATE}_rustbuffer_from_bytes`,
};
```

### Example: Complete register() for async_add

This shows the full register() definitions needed for a single async function call, demonstrating all the moving parts. The exact symbol names will be verified via `nm -gU` after the first build and corrected as needed.

```js
const nm = lib.register({
  symbols: SYMBOLS,
  structs: {},
  callbacks: {
    // The continuation callback: (handle, pollResult) -> void
    // Registered once, used by all async poll calls.
    rust_future_continuation: {
      args: [FfiType.UInt64, FfiType.Int8],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
  },
  functions: {
    // Initiate the async call — returns a future handle (u64)
    [`uniffi_${CRATE}_fn_func_async_add`]: {
      args: [FfiType.UInt32, FfiType.UInt32],
      ret: FfiType.Handle,
      hasRustCallStatus: true,
    },
    // Poll the future — takes (future_handle, continuation_callback, callback_data)
    [`uniffi_${CRATE}_rust_future_poll_u32`]: {
      args: [FfiType.Handle, FfiType.Callback('rust_future_continuation'), FfiType.UInt64],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
    // Extract result after POLL_READY
    [`uniffi_${CRATE}_rust_future_complete_u32`]: {
      args: [FfiType.Handle],
      ret: FfiType.UInt32,
      hasRustCallStatus: true,
    },
    // Free the future handle
    [`uniffi_${CRATE}_rust_future_free_u32`]: {
      args: [FfiType.Handle],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
  },
});
```

**Note on continuation callback registration:** UniFFI may use either a per-call approach (callback passed as an argument to `poll`) or a global `continuation_callback_set` function. The above assumes per-call. If UniFFI 0.31 uses the global approach, the continuation callback would be registered once via a `continuation_callback_set` function and the poll signature would change. We will verify this empirically.

**Note on per-call callback leak:** If the continuation callback is passed per-poll-call as a `FfiType.Callback`, uniffi-napi creates and leaks a new libffi closure each time (by design — see `register.rs`). For short-lived futures that poll once, this is negligible. For long-polling futures, a global registration approach would be preferable. This is a known trade-off that a future optimization can address.

### Example: Calculator VTable register() definitions

At the FFI boundary, UniFFI passes trait objects as opaque `u64` handles. The `Box<dyn Calculator>` in Rust becomes `FfiType.Handle` in the register definitions.

```js
structs: {
  // VTable struct — fields must match C struct order.
  // UniFFI generates: each trait method + uniffi_free.
  VTable_Calculator: [
    { name: 'add', type: FfiType.Callback('callback_calculator_add') },
    { name: 'concatenate', type: FfiType.Callback('callback_calculator_concatenate') },
    { name: 'uniffi_free', type: FfiType.Callback('callback_calculator_free') },
  ],
},
callbacks: {
  callback_calculator_add: {
    // (handle, a, b, &mut RustCallStatus) -> u32
    args: [FfiType.UInt64, FfiType.UInt32, FfiType.UInt32],
    ret: FfiType.UInt32,
    hasRustCallStatus: true,
  },
  callback_calculator_concatenate: {
    // (handle, a, b, &mut RustCallStatus) -> RustBuffer
    args: [FfiType.UInt64, FfiType.RustBuffer, FfiType.RustBuffer],
    ret: FfiType.RustBuffer,
    hasRustCallStatus: true,
  },
  callback_calculator_free: {
    args: [FfiType.UInt64],
    ret: FfiType.Void,
    hasRustCallStatus: true,
  },
},
functions: {
  [`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]: {
    args: [FfiType.Reference(FfiType.Struct('VTable_Calculator'))],
    ret: FfiType.Void,
    hasRustCallStatus: true,
  },
  [`uniffi_${CRATE}_fn_func_use_calculator`]: {
    // Box<dyn Calculator> is passed as a Handle (u64) at the FFI boundary
    args: [FfiType.Handle, FfiType.UInt32, FfiType.UInt32],
    ret: FfiType.UInt32,
    hasRustCallStatus: true,
  },
  [`uniffi_${CRATE}_fn_func_use_calculator_from_thread`]: {
    // Same signature — but internally spawns a std::thread,
    // exercising the blocking cross-thread VTable dispatch.
    args: [FfiType.Handle, FfiType.UInt32, FfiType.UInt32],
    ret: FfiType.UInt32,
    hasRustCallStatus: true,
  },
},
```

### AsyncFetcher VTable (async foreign trait)

The `#[uniffi::export(with_foreign)]` async trait generates a more complex VTable. The exact protocol depends on the UniFFI version — the VTable method may return a `ForeignFuture` struct (handle + free function pointer) rather than the result directly. The `ForeignFuture` protocol allows Rust to poll the JS-owned async operation.

**This is the most complex part of the design.** The exact VTable layout, return types, and polling protocol will be determined empirically after building the fixture. If the foreign future protocol proves incompatible with uniffi-napi's current struct/callback support, test #9 will be deferred to a follow-up that adds the necessary infrastructure.

---

## Build & Test Flow

```bash
# 1. Build the fixture crate
cd fixtures/uniffi-fixture-simple && cargo build

# 2. Verify symbols (one-time, to confirm naming)
nm -gU target/debug/libuniffi_fixture_simple.dylib | grep uniffi

# 3. Build uniffi-napi (if not already)
cd ../.. && npm run build:debug

# 4. Run tests
npm test
```

The fixture test is included in the existing `node --test tests/*.test.mjs` glob.

---

## Risk: Symbol Name Discovery

UniFFI's internal symbol naming conventions are not part of its public API. The exact names depend on:
- The crate name (underscores in `Cargo.toml` `[package].name`)
- The function/trait/type names
- UniFFI version (we target 0.31)

**Mitigation:** After the first successful build, run `nm -gU` to list all exported symbols and correct any naming mismatches. This is a one-time manual step.

## Risk: Async Callback (Foreign Future) Complexity

The async callback path (test #9) requires understanding UniFFI's foreign future protocol — how Rust polls a JS-owned future. This is the most complex test and may require additional scaffolding symbols.

**Mitigation:** Start with sync tests (1-9), get them passing, then tackle the async callback. If the foreign future protocol proves too complex for this initial fixture, defer test #10 to a follow-up.

---

## Success Criteria

All 10 tests pass, proving that uniffi-napi correctly:
- Calls real UniFFI-generated scaffolding functions
- Marshals primitives (u32, f64) and strings (via RustBuffer) in both directions
- Handles errors with serialized error buffers
- Polls async Rust futures via continuation callbacks
- Dispatches sync VTable callbacks on the main thread (foreign trait methods)
- Dispatches sync VTable callbacks from a worker thread (blocking cross-thread)
- Dispatches async VTable callbacks (foreign futures)
