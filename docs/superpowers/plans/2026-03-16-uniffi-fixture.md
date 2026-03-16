# UniFFI Fixture Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a real UniFFI fixture crate and end-to-end JS tests proving uniffi-napi works with UniFFI-generated scaffolding — sync/async calls, callbacks, cross-thread dispatch, primitives, strings, and errors.

**Architecture:** A `fixtures/uniffi-fixture-simple/` Rust crate using `#[uniffi::export]` proc macros generates real C scaffolding. Two new `lib/` modules (`async.js`, `converters.js`) provide reusable runtime helpers. A single `tests/fixture.test.mjs` hand-writes `register()` definitions matching UniFFI's symbol names and runs 10 integration tests.

**Tech Stack:** UniFFI 0.31 (proc macros), napi-rs, libffi, Node.js test runner, thiserror

**Spec:** `docs/superpowers/specs/2026-03-16-uniffi-fixture-design.md`

---

## File Structure

### New files

| File | Responsibility |
|------|---------------|
| `fixtures/uniffi-fixture-simple/Cargo.toml` | UniFFI fixture crate manifest |
| `fixtures/uniffi-fixture-simple/src/lib.rs` | Fixture Rust code: sync/async functions, traits, error types |
| `tests/helpers/converters.mjs` | String lift/lower (UniFFI wire format: 4-byte BE length + UTF-8) |
| `tests/helpers/async.mjs` | Async runtime: HandleMap, continuation callback, polling loop |
| `tests/fixture.test.mjs` | Integration tests for all 10 test cases |

### Modified files

| File | Change |
|------|--------|
| `.gitignore` | Add `fixtures/*/target/` |

---

## Chunk 1: Fixture Crate + Runtime Libraries

### Task 1: Create the fixture crate and verify it compiles

**Files:**
- Create: `fixtures/uniffi-fixture-simple/Cargo.toml`
- Create: `fixtures/uniffi-fixture-simple/src/lib.rs`
- Modify: `.gitignore`

- [ ] **Step 1: Create Cargo.toml**

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

- [ ] **Step 2: Create src/lib.rs with sync functions only (minimal first)**

Start with just the sync functions to verify UniFFI compiles:

```rust
uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ArithmeticError {
    #[error("{reason}")]
    DivisionByZero { reason: String },
}

#[uniffi::export]
pub fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[uniffi::export]
pub fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

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
```

- [ ] **Step 3: Add fixtures target to .gitignore**

Add `fixtures/*/target/` to `.gitignore`.

- [ ] **Step 4: Build and verify symbols**

```bash
cd fixtures/uniffi-fixture-simple && cargo build
```

Expected: Compiles successfully, producing `target/debug/libuniffi_fixture_simple.dylib` (macOS) or `.so` (Linux).

Then verify exported symbols:

```bash
nm -gU target/debug/libuniffi_fixture_simple.dylib | grep "uniffi_uniffi_fixture_simple"
```

Expected: At least these symbols visible:
- `uniffi_uniffi_fixture_simple_fn_func_add`
- `uniffi_uniffi_fixture_simple_fn_func_greet`
- `uniffi_uniffi_fixture_simple_fn_func_divide`
- `uniffi_uniffi_fixture_simple_rustbuffer_alloc`
- `uniffi_uniffi_fixture_simple_rustbuffer_free`
- `uniffi_uniffi_fixture_simple_rustbuffer_from_bytes`

**Record the exact symbol names** — if they differ from predictions, note the corrections for use in later tasks.

- [ ] **Step 5: Commit**

```bash
git add fixtures/uniffi-fixture-simple/ .gitignore
git commit -m "feat: add UniFFI fixture crate with sync functions and error type"
```

---

### Task 2: Add async functions to the fixture

**Files:**
- Modify: `fixtures/uniffi-fixture-simple/src/lib.rs`

- [ ] **Step 1: Add async functions**

Append to `src/lib.rs`:

```rust
#[uniffi::export]
pub async fn async_add(a: u32, b: u32) -> u32 {
    a + b
}

#[uniffi::export]
pub async fn async_greet(name: String) -> String {
    format!("Hello, {name}!")
}
```

- [ ] **Step 2: Build and verify async symbols**

```bash
cd fixtures/uniffi-fixture-simple && cargo build
```

If UniFFI requires a Tokio runtime dependency, add it:
```toml
tokio = { version = "1", features = ["rt"] }
```

Verify new symbols:
```bash
nm -gU target/debug/libuniffi_fixture_simple.dylib | grep "rust_future"
```

Expected: Per-return-type poll/complete/free/cancel symbols, plus possibly a continuation callback set symbol. **Record the exact names.**

- [ ] **Step 3: Commit**

```bash
git add fixtures/uniffi-fixture-simple/
git commit -m "feat: add async functions to UniFFI fixture"
```

---

### Task 3: Add callback interface and cross-thread function

**Files:**
- Modify: `fixtures/uniffi-fixture-simple/src/lib.rs`

- [ ] **Step 1: Add Calculator trait and functions**

Append to `src/lib.rs`:

```rust
#[uniffi::export(callback_interface)]
pub trait Calculator {
    fn add(&self, a: u32, b: u32) -> u32;
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

#[uniffi::export]
pub async fn use_calculator_from_thread(calc: Box<dyn Calculator>, a: u32, b: u32) -> u32 {
    let handle = tokio::runtime::Handle::current();
    handle.spawn_blocking(move || calc.add(a, b)).await.unwrap()
}
```

- [ ] **Step 2: Build and verify callback symbols**

```bash
cd fixtures/uniffi-fixture-simple && cargo build
nm -gU target/debug/libuniffi_fixture_simple.dylib | grep "calculator"
```

Expected: VTable init symbol and function symbols. **Record exact names and VTable struct field order** — this is critical for the register() definitions.

- [ ] **Step 3: Commit**

```bash
git add fixtures/uniffi-fixture-simple/
git commit -m "feat: add Calculator callback interface with cross-thread test"
```

---

### Task 4: Add async foreign trait (AsyncFetcher)

**Files:**
- Modify: `fixtures/uniffi-fixture-simple/Cargo.toml` (may need `async-trait`)
- Modify: `fixtures/uniffi-fixture-simple/src/lib.rs`

- [ ] **Step 1: Add AsyncFetcher trait**

Append to `src/lib.rs`. Try without `async-trait` first (UniFFI 0.31 may handle it natively):

```rust
#[uniffi::export(with_foreign)]
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

If this doesn't compile, try adding `#[async_trait::async_trait]` and `async-trait = "0.1"` to Cargo.toml.

- [ ] **Step 2: Build and verify symbols**

```bash
cd fixtures/uniffi-fixture-simple && cargo build
nm -gU target/debug/libuniffi_fixture_simple.dylib | grep -i "fetcher\|foreign_future"
```

**Record all symbol names.** The async foreign trait VTable is the most complex part — note the exact struct layout and any additional scaffolding symbols.

- [ ] **Step 3: Commit**

```bash
git add fixtures/uniffi-fixture-simple/
git commit -m "feat: add AsyncFetcher foreign trait to fixture"
```

---

### Task 5: Create tests/helpers/converters.mjs

**Files:**
- Create: `tests/helpers/converters.mjs`

- [ ] **Step 1: Write converters.mjs (ESM)**

```js
// UniFFI wire-format serialization helpers.
//
// UniFFI serializes compound types into RustBuffer using a simple
// binary format: each value is preceded by a length or tag, all
// integers are big-endian. These helpers convert between JS values
// and Uint8Array buffers matching that format.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * Lower a JS string into a Uint8Array matching UniFFI's string wire format:
 * 4-byte big-endian Int32 byte length, followed by UTF-8 bytes.
 */
export function lowerString(s) {
  const encoded = encoder.encode(s);
  const buf = new Uint8Array(4 + encoded.length);
  new DataView(buf.buffer).setInt32(0, encoded.length, false);
  buf.set(encoded, 4);
  return buf;
}

/**
 * Lift a Uint8Array (from a RustBuffer return) into a JS string.
 * Reads: 4-byte big-endian Int32 byte length, then that many UTF-8 bytes.
 */
export function liftString(buf) {
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const len = view.getInt32(0, false);
  return decoder.decode(buf.slice(4, 4 + len));
}

/**
 * Lift a UniFFI error enum from a Uint8Array (from RustCallStatus.errorBuf).
 * Reads: 4-byte big-endian Int32 variant index, then variant fields.
 * Returns { variant: number, ...fields }.
 *
 * For ArithmeticError::DivisionByZero { reason: String }:
 *   variant=1, reason=liftString(remaining bytes)
 */
export function liftArithmeticError(buf) {
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  const variant = view.getInt32(0, false);
  const reason = liftString(buf.slice(4));
  return { variant, reason };
}
```

- [ ] **Step 2: Commit**

```bash
git add tests/helpers/converters.mjs
git commit -m "feat: add UniFFI wire-format string converters"
```

---

### Task 6: Create tests/helpers/async.mjs

**Files:**
- Create: `tests/helpers/async.mjs`

- [ ] **Step 1: Write async.mjs (ESM)**

```js
// Async runtime for polling UniFFI Rust futures.
//
// UniFFI async functions return a future handle (u64). The caller must
// poll the future with a continuation callback until it signals READY,
// then call complete() to extract the result and free() to drop the handle.
//
// The continuation callback is a fire-and-forget (void, no RustCallStatus)
// function invoked by Rust — possibly from a worker thread — with
// (handle: u64, pollResult: i8). We use a HandleMap to correlate each
// poll call with its Promise resolver.

const POLL_READY = 0;

/**
 * Maps bigint handles to Promise resolve functions.
 * Each in-flight poll gets a unique handle; the continuation callback
 * looks up the resolver by handle and calls it with the poll result.
 */
class HandleMap {
  #nextHandle = 1n;
  #map = new Map();

  insert(value) {
    const handle = this.#nextHandle;
    this.#nextHandle += 1n;
    this.#map.set(handle, value);
    return handle;
  }

  remove(handle) {
    const value = this.#map.get(handle);
    this.#map.delete(handle);
    return value;
  }
}

const resolverMap = new HandleMap();

/**
 * The continuation callback passed to Rust's poll function.
 * Signature at the C level: (handle: u64, pollResult: i8) -> void.
 * hasRustCallStatus: false (fire-and-forget).
 *
 * When Rust finishes a poll iteration, it calls this from whichever
 * thread the future was polled on. uniffi-napi's NonBlocking TSF
 * dispatch delivers the call to the main thread's event loop.
 */
function continuationCallback(handle, pollResult) {
  const resolve = resolverMap.remove(handle);
  if (resolve) resolve(pollResult);
}

/**
 * Poll a UniFFI Rust future to completion and return the result.
 *
 * @param {object} nm - The registered native module (from register())
 * @param {object} opts
 * @param {function} opts.rustFutureFunc - () => bigint: initiates the async call, returns future handle
 * @param {string} opts.pollFunc - Symbol name for the poll function
 * @param {string} opts.completeFunc - Symbol name for the complete function
 * @param {string} opts.freeFunc - Symbol name for the free function
 * @param {function} [opts.liftFunc] - (rawResult) => T: converts the raw return value to JS
 * @param {object} [opts.callStatus] - { code: 0 } RustCallStatus object for complete()
 * @returns {Promise<*>} The lifted result
 */
async function uniffiRustCallAsync(nm, {
  rustFutureFunc,
  pollFunc,
  completeFunc,
  freeFunc,
  liftFunc,
  callStatus,
}) {
  const futureHandle = rustFutureFunc();
  const status = callStatus || { code: 0 };

  try {
    let pollResult;
    do {
      pollResult = await new Promise((resolve) => {
        const handle = resolverMap.insert(resolve);
        nm[pollFunc](futureHandle, continuationCallback, handle);
      });
    } while (pollResult !== POLL_READY);

    const result = nm[completeFunc](futureHandle, status);
    if (status.code !== 0) {
      throw new Error(`Rust async call failed with code ${status.code}`);
    }
    return liftFunc ? liftFunc(result) : result;
  } finally {
    nm[freeFunc](futureHandle);
  }
}

export {
  POLL_READY,
  HandleMap,
  resolverMap,
  continuationCallback,
  uniffiRustCallAsync,
};
```

- [ ] **Step 2: Commit**

```bash
git add tests/helpers/async.mjs
git commit -m "feat: add async runtime for polling UniFFI Rust futures"
```

---

## Chunk 2: Sync Tests (Tests 1-4)

### Task 7: Write sync scalar test (test #1: add)

**Files:**
- Create: `tests/fixture.test.mjs`

This task creates the test file with shared setup and the first test. The `register()` definitions will start minimal and grow as we add tests.

**Important:** The symbol names below are predictions based on UniFFI conventions. If Task 1 Step 4 revealed different names, use those instead.

- [ ] **Step 1: Create tests/fixture.test.mjs with shared setup and first test**

```js
import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import lib from '../lib.js';
const { UniffiNativeModule, FfiType } = lib;

const LIB_PATH = join(import.meta.dirname, '..', 'fixtures', 'uniffi-fixture-simple',
  'target', 'debug',
  process.platform === 'darwin' ? 'libuniffi_fixture_simple.dylib' : 'libuniffi_fixture_simple.so'
);

const CRATE = 'uniffi_fixture_simple';

const SYMBOLS = {
  rustbufferAlloc: `uniffi_${CRATE}_rustbuffer_alloc`,
  rustbufferFree: `uniffi_${CRATE}_rustbuffer_free`,
  rustbufferFromBytes: `uniffi_${CRATE}_rustbuffer_from_bytes`,
};

function openAndRegister(extraFunctions = {}, extraCallbacks = {}, extraStructs = {}) {
  const mod = UniffiNativeModule.open(LIB_PATH);
  return mod.register({
    symbols: SYMBOLS,
    structs: extraStructs,
    callbacks: extraCallbacks,
    functions: extraFunctions,
  });
}

test('fixture: add(3, 4) = 7 (sync scalar)', () => {
  const nm = openAndRegister({
    [`uniffi_${CRATE}_fn_func_add`]: {
      args: [FfiType.UInt32, FfiType.UInt32],
      ret: FfiType.UInt32,
      hasRustCallStatus: true,
    },
  });

  const status = { code: 0 };
  const result = nm[`uniffi_${CRATE}_fn_func_add`](3, 4, status);
  assert.strictEqual(status.code, 0);
  assert.strictEqual(result, 7);
});
```

- [ ] **Step 2: Build uniffi-napi if needed and run test**

```bash
npm run build:debug  # if needed
npm test -- --test-name-pattern "fixture: add"
```

Expected: PASS. If it fails due to symbol name mismatch, correct the symbol name based on the `nm` output from Task 1.

- [ ] **Step 3: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: fixture sync scalar add(3,4)=7"
```

---

### Task 8: Write sync string test (test #2: greet)

**Files:**
- Modify: `tests/fixture.test.mjs`

- [ ] **Step 1: Add import for converters and greet test**

Add at top of file:
```js
import { lowerString, liftString, liftArithmeticError } from './helpers/converters.mjs';
```

Add test:
```js
test('fixture: greet("World") = "Hello, World!" (sync string)', () => {
  const nm = openAndRegister({
    [`uniffi_${CRATE}_fn_func_greet`]: {
      args: [FfiType.RustBuffer],
      ret: FfiType.RustBuffer,
      hasRustCallStatus: true,
    },
  });

  const status = { code: 0 };
  const result = nm[`uniffi_${CRATE}_fn_func_greet`](lowerString('World'), status);
  assert.strictEqual(status.code, 0);
  assert.strictEqual(liftString(result), 'Hello, World!');
});
```

- [ ] **Step 2: Run test**

```bash
npm test -- --test-name-pattern "fixture: greet"
```

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: fixture sync string greet round-trip"
```

---

### Task 9: Write sync error tests (tests #3 and #4: divide)

**Files:**
- Modify: `tests/fixture.test.mjs`

- [ ] **Step 1: Add divide error test**

```js
test('fixture: divide(1.0, 0.0) returns error (sync error path)', () => {
  const nm = openAndRegister({
    [`uniffi_${CRATE}_fn_func_divide`]: {
      args: [FfiType.Float64, FfiType.Float64],
      ret: FfiType.Float64,
      hasRustCallStatus: true,
    },
  });

  const status = { code: 0 };
  nm[`uniffi_${CRATE}_fn_func_divide`](1.0, 0.0, status);
  assert.notStrictEqual(status.code, 0, 'Expected non-zero error code');
  assert.ok(status.errorBuf instanceof Uint8Array, 'Expected errorBuf');

  const error = liftArithmeticError(status.errorBuf);
  assert.strictEqual(error.variant, 1); // DivisionByZero
  assert.ok(error.reason.includes('cannot divide by zero'));
});
```

- [ ] **Step 2: Add divide success test**

```js
test('fixture: divide(10.0, 2.0) = 5.0 (sync success path)', () => {
  const nm = openAndRegister({
    [`uniffi_${CRATE}_fn_func_divide`]: {
      args: [FfiType.Float64, FfiType.Float64],
      ret: FfiType.Float64,
      hasRustCallStatus: true,
    },
  });

  const status = { code: 0 };
  const result = nm[`uniffi_${CRATE}_fn_func_divide`](10.0, 2.0, status);
  assert.strictEqual(status.code, 0);
  assert.strictEqual(result, 5.0);
});
```

- [ ] **Step 3: Run tests**

```bash
npm test -- --test-name-pattern "fixture: divide"
```

Expected: Both PASS. The error test validates that `callStatus.errorBuf` contains a serialized `ArithmeticError`.

- [ ] **Step 4: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: fixture sync divide — error and success paths"
```

---

## Chunk 3: Async Tests (Tests 5-6)

### Task 10: Write async scalar test (test #5: async_add)

**Files:**
- Modify: `tests/fixture.test.mjs`

**Important:** The poll/complete/free symbol names and the continuation callback registration pattern must match what `nm -gU` showed in Task 2. The code below uses predicted names — adjust as needed.

- [ ] **Step 1: Add import for async runtime**

Add at top of file:
```js
import { continuationCallback, uniffiRustCallAsync } from './helpers/async.mjs';
```

- [ ] **Step 2: Add async_add test**

```js
test('fixture: async_add(3, 4) = 7 (async scalar)', async () => {
  const nm = openAndRegister({
    [`uniffi_${CRATE}_fn_func_async_add`]: {
      args: [FfiType.UInt32, FfiType.UInt32],
      ret: FfiType.Handle,
      hasRustCallStatus: true,
    },
    [`uniffi_${CRATE}_rust_future_poll_u32`]: {
      args: [FfiType.Handle, FfiType.Callback('rust_future_continuation'), FfiType.UInt64],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
    [`uniffi_${CRATE}_rust_future_complete_u32`]: {
      args: [FfiType.Handle],
      ret: FfiType.UInt32,
      hasRustCallStatus: true,
    },
    [`uniffi_${CRATE}_rust_future_free_u32`]: {
      args: [FfiType.Handle],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
  }, {
    rust_future_continuation: {
      args: [FfiType.UInt64, FfiType.Int8],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
  });

  const result = await uniffiRustCallAsync(nm, {
    rustFutureFunc: () => {
      const status = { code: 0 };
      return nm[`uniffi_${CRATE}_fn_func_async_add`](3, 4, status);
    },
    pollFunc: `uniffi_${CRATE}_rust_future_poll_u32`,
    completeFunc: `uniffi_${CRATE}_rust_future_complete_u32`,
    freeFunc: `uniffi_${CRATE}_rust_future_free_u32`,
  });

  assert.strictEqual(result, 7);
});
```

- [ ] **Step 3: Run test**

```bash
npm test -- --test-name-pattern "fixture: async_add"
```

Expected: PASS. This is the first async test — if the symbol names or continuation callback pattern don't match, this is where it will fail. Debug by checking:
1. Do the symbol names match `nm` output?
2. Is the continuation callback signature correct?
3. Does the poll function expect the callback as an argument or registered globally?

- [ ] **Step 4: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: fixture async scalar async_add(3,4)=7"
```

---

### Task 11: Write async string test (test #6: async_greet)

**Files:**
- Modify: `tests/fixture.test.mjs`

- [ ] **Step 1: Add async_greet test**

The RustBuffer return type uses different poll/complete/free symbols than u32. Check `nm` output for the exact names (likely `_rust_buffer` suffix).

```js
test('fixture: async_greet("World") = "Hello, World!" (async string)', async () => {
  const nm = openAndRegister({
    [`uniffi_${CRATE}_fn_func_async_greet`]: {
      args: [FfiType.RustBuffer],
      ret: FfiType.Handle,
      hasRustCallStatus: true,
    },
    [`uniffi_${CRATE}_rust_future_poll_rust_buffer`]: {
      args: [FfiType.Handle, FfiType.Callback('rust_future_continuation'), FfiType.UInt64],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
    [`uniffi_${CRATE}_rust_future_complete_rust_buffer`]: {
      args: [FfiType.Handle],
      ret: FfiType.RustBuffer,
      hasRustCallStatus: true,
    },
    [`uniffi_${CRATE}_rust_future_free_rust_buffer`]: {
      args: [FfiType.Handle],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
  }, {
    rust_future_continuation: {
      args: [FfiType.UInt64, FfiType.Int8],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
  });

  const result = await uniffiRustCallAsync(nm, {
    rustFutureFunc: () => {
      const status = { code: 0 };
      return nm[`uniffi_${CRATE}_fn_func_async_greet`](lowerString('World'), status);
    },
    pollFunc: `uniffi_${CRATE}_rust_future_poll_rust_buffer`,
    completeFunc: `uniffi_${CRATE}_rust_future_complete_rust_buffer`,
    freeFunc: `uniffi_${CRATE}_rust_future_free_rust_buffer`,
    liftFunc: liftString,
  });

  assert.strictEqual(result, 'Hello, World!');
});
```

- [ ] **Step 2: Run test**

```bash
npm test -- --test-name-pattern "fixture: async_greet"
```

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: fixture async string async_greet round-trip"
```

---

## Chunk 4: Callback Tests (Tests 7-9)

### Task 12: Write sync callback tests (tests #7-8: Calculator)

**Files:**
- Modify: `tests/fixture.test.mjs`

**Important:** The VTable struct field names and callback names must match what UniFFI generates. Verify with `nm -gU` output from Task 3. The names below are predictions.

- [ ] **Step 1: Add Calculator VTable tests**

```js
test('fixture: Calculator.add via VTable (sync callback, scalar)', () => {
  const nm = openAndRegister(
    {
      [`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]: {
        args: [FfiType.Reference(FfiType.Struct('VTable_Calculator'))],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
      [`uniffi_${CRATE}_fn_func_use_calculator`]: {
        args: [FfiType.Handle, FfiType.UInt32, FfiType.UInt32],
        ret: FfiType.UInt32,
        hasRustCallStatus: true,
      },
    },
    {
      callback_calculator_add: {
        args: [FfiType.UInt64, FfiType.UInt32, FfiType.UInt32],
        ret: FfiType.UInt32,
        hasRustCallStatus: true,
      },
      callback_calculator_concatenate: {
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
    {
      VTable_Calculator: [
        { name: 'add', type: FfiType.Callback('callback_calculator_add') },
        { name: 'concatenate', type: FfiType.Callback('callback_calculator_concatenate') },
        { name: 'uniffi_free', type: FfiType.Callback('callback_calculator_free') },
      ],
    },
  );

  // Register VTable
  const status1 = { code: 0 };
  nm[`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]({
    add: (handle, a, b, callStatus) => {
      callStatus.code = 0;
      return a + b;
    },
    concatenate: (handle, aBuf, bBuf, callStatus) => {
      callStatus.code = 0;
      return lowerString(liftString(aBuf) + liftString(bBuf));
    },
    uniffi_free: (handle, callStatus) => {
      callStatus.code = 0;
    },
  }, status1);
  assert.strictEqual(status1.code, 0);

  // Call through VTable
  const status2 = { code: 0 };
  const result = nm[`uniffi_${CRATE}_fn_func_use_calculator`](1n, 3, 4, status2);
  assert.strictEqual(status2.code, 0);
  assert.strictEqual(result, 7);
});
```

- [ ] **Step 2: Add concatenate test (test #8)**

```js
test('fixture: Calculator.concatenate via VTable (sync callback, string)', () => {
  // Reuse the same registration pattern as above but test concatenate.
  // This requires a fresh register() since VTable init is per-register.
  const nm = openAndRegister(
    {
      [`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]: {
        args: [FfiType.Reference(FfiType.Struct('VTable_Calculator'))],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
      [`uniffi_${CRATE}_fn_func_use_calculator_strings`]: {
        args: [FfiType.Handle, FfiType.RustBuffer, FfiType.RustBuffer],
        ret: FfiType.RustBuffer,
        hasRustCallStatus: true,
      },
    },
    {
      callback_calculator_add: {
        args: [FfiType.UInt64, FfiType.UInt32, FfiType.UInt32],
        ret: FfiType.UInt32,
        hasRustCallStatus: true,
      },
      callback_calculator_concatenate: {
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
    {
      VTable_Calculator: [
        { name: 'add', type: FfiType.Callback('callback_calculator_add') },
        { name: 'concatenate', type: FfiType.Callback('callback_calculator_concatenate') },
        { name: 'uniffi_free', type: FfiType.Callback('callback_calculator_free') },
      ],
    },
  );

  const status1 = { code: 0 };
  nm[`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]({
    add: (handle, a, b, callStatus) => {
      callStatus.code = 0;
      return a + b;
    },
    concatenate: (handle, aBuf, bBuf, callStatus) => {
      callStatus.code = 0;
      // aBuf and bBuf are Uint8Array (serialized strings).
      // Lift them, concatenate in JS, lower the result.
      const a = liftString(aBuf);
      const b = liftString(bBuf);
      return lowerString(a + b);
    },
    uniffi_free: (handle, callStatus) => {
      callStatus.code = 0;
    },
  }, status1);
  assert.strictEqual(status1.code, 0);

  const status2 = { code: 0 };
  const result = nm[`uniffi_${CRATE}_fn_func_use_calculator_strings`](
    1n, lowerString('hello'), lowerString(' world'), status2
  );
  assert.strictEqual(status2.code, 0);
  assert.strictEqual(liftString(result), 'hello world');
});
```

- [ ] **Step 3: Run tests**

```bash
npm test -- --test-name-pattern "fixture: Calculator"
```

Expected: Both PASS. If the VTable field names or callback signatures don't match, debug with `nm -gU` output.

- [ ] **Step 4: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: fixture Calculator VTable — scalar and string callbacks"
```

---

### Task 13: Write cross-thread callback test (test #9)

**Files:**
- Modify: `tests/fixture.test.mjs`

`use_calculator_from_thread` is an **async** function — it must be, because a sync version would deadlock (the main thread would block on `.join()` while the VTable callback needs the main thread to process via TSF). The async scaffolding frees the main thread between poll iterations.

This test combines the async polling loop with a VTable callback, exercising the full cross-thread path: JS calls async Rust function → Rust spawns a blocking thread → blocking thread calls VTable callback → callback dispatches to JS main thread via TSF → result flows back through the channel → async future completes.

- [ ] **Step 1: Add cross-thread Calculator test**

This requires both the Calculator VTable definitions AND the async polling infrastructure (continuation callback + poll/complete/free for u32).

```js
test('fixture: Calculator.add from background thread (async + cross-thread VTable)', async () => {
  const nm = openAndRegister(
    {
      [`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]: {
        args: [FfiType.Reference(FfiType.Struct('VTable_Calculator'))],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
      // This is an async function — returns a future handle
      [`uniffi_${CRATE}_fn_func_use_calculator_from_thread`]: {
        args: [FfiType.Handle, FfiType.UInt32, FfiType.UInt32],
        ret: FfiType.Handle,
        hasRustCallStatus: true,
      },
      // Async polling infrastructure (u32 return type)
      [`uniffi_${CRATE}_rust_future_poll_u32`]: {
        args: [FfiType.Handle, FfiType.Callback('rust_future_continuation'), FfiType.UInt64],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      [`uniffi_${CRATE}_rust_future_complete_u32`]: {
        args: [FfiType.Handle],
        ret: FfiType.UInt32,
        hasRustCallStatus: true,
      },
      [`uniffi_${CRATE}_rust_future_free_u32`]: {
        args: [FfiType.Handle],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
    },
    {
      rust_future_continuation: {
        args: [FfiType.UInt64, FfiType.Int8],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      callback_calculator_add: {
        args: [FfiType.UInt64, FfiType.UInt32, FfiType.UInt32],
        ret: FfiType.UInt32,
        hasRustCallStatus: true,
      },
      callback_calculator_concatenate: {
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
    {
      VTable_Calculator: [
        { name: 'add', type: FfiType.Callback('callback_calculator_add') },
        { name: 'concatenate', type: FfiType.Callback('callback_calculator_concatenate') },
        { name: 'uniffi_free', type: FfiType.Callback('callback_calculator_free') },
      ],
    },
  );

  // Register VTable
  const status1 = { code: 0 };
  nm[`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]({
    add: (handle, a, b, callStatus) => {
      callStatus.code = 0;
      return a + b;
    },
    concatenate: (handle, aBuf, bBuf, callStatus) => {
      callStatus.code = 0;
      return lowerString(liftString(aBuf) + liftString(bBuf));
    },
    uniffi_free: (handle, callStatus) => {
      callStatus.code = 0;
    },
  }, status1);
  assert.strictEqual(status1.code, 0);

  // Call the async function via polling loop.
  // Internally Rust spawns a blocking thread that calls calc.add()
  // through the VTable — cross-thread dispatch via TSF.
  const result = await uniffiRustCallAsync(nm, {
    rustFutureFunc: () => {
      const status = { code: 0 };
      return nm[`uniffi_${CRATE}_fn_func_use_calculator_from_thread`](1n, 3, 4, status);
    },
    pollFunc: `uniffi_${CRATE}_rust_future_poll_u32`,
    completeFunc: `uniffi_${CRATE}_rust_future_complete_u32`,
    freeFunc: `uniffi_${CRATE}_rust_future_free_u32`,
  });

  assert.strictEqual(result, 7);
});
```

- [ ] **Step 2: Run test**

```bash
npm test -- --test-name-pattern "fixture.*cross-thread"
```

Expected: PASS. This exercises the full cross-thread path: async Rust function → `spawn_blocking` → VTable callback via TSF → result back through channel → future completes.

- [ ] **Step 3: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: fixture cross-thread Calculator VTable callback (async)"
```

---

## Chunk 5: Async Callback Test (Test 10)

### Task 14: Write async foreign trait test (test #10: AsyncFetcher)

**Files:**
- Modify: `tests/fixture.test.mjs`

**This is the most complex test.** The AsyncFetcher's VTable uses UniFFI's foreign future protocol, which may involve additional struct types (ForeignFuture) and callback signatures not yet supported by uniffi-napi. The exact protocol depends on the `nm -gU` output from Task 4.

- [ ] **Step 1: Examine nm output from Task 4 for AsyncFetcher symbols**

Review the symbol list for anything related to `fetcher`, `foreign_future`, or `async_fetcher`. Identify:
- The VTable init symbol name
- The VTable struct field names and callback signatures
- Any additional struct types (ForeignFuture may be a struct with {handle, free} fields)
- The `use_async_fetcher` function signature (it's async, so it also needs poll/complete/free)

- [ ] **Step 2: Write the test based on discovered symbols**

The test structure will be:
1. Register the AsyncFetcher VTable with a JS implementation
2. Call `use_async_fetcher` (an async function) via `uniffiRustCallAsync`
3. When Rust calls `fetcher.fetch()`, the VTable callback fires on the JS side
4. The JS callback returns a result (possibly via the foreign future protocol)
5. Assert the final result

The exact code depends on the symbol discovery in Step 1. Write the test matching the actual symbols.

**If the foreign future protocol requires struct types not yet supported by uniffi-napi** (e.g., a `ForeignFuture` struct with non-callback fields), document what's missing and skip this test with a clear TODO comment explaining what infrastructure needs to be added.

- [ ] **Step 3: Run test**

```bash
npm test -- --test-name-pattern "fixture.*AsyncFetcher"
```

- [ ] **Step 4: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: fixture AsyncFetcher foreign trait (async callback)"
```

---

## Chunk 6: Final Verification

### Task 15: Run all tests and verify

- [ ] **Step 1: Run the full test suite**

```bash
cd /Users/jhugman/workspaces/uniffi-napi
npm test
```

Expected: All existing tests (33) still pass, plus the new fixture tests (up to 10).

- [ ] **Step 2: Verify no regressions in existing tests**

If any existing test fails, it was broken by a change to `lib.js` or the native addon. Investigate and fix.

- [ ] **Step 3: Final commit if any fixups needed**

```bash
git add -A
git commit -m "fix: address test regressions from fixture integration"
```

---

## Notes for the Implementer

### Symbol Name Discovery

The predicted symbol names in this plan follow UniFFI 0.31 conventions but **may not be exact**. After each `cargo build`, run:

```bash
nm -gU fixtures/uniffi-fixture-simple/target/debug/libuniffi_fixture_simple.dylib | grep uniffi
```

And correct the symbol names in the test code. This is expected and not a bug.

### Debugging Async Tests

If async tests hang or timeout:
1. Check that the continuation callback signature matches what Rust expects
2. Check whether UniFFI uses per-call or global callback registration
3. Add `console.log` in the continuation callback to verify it fires
4. Check that `pollResult` values match (POLL_READY = 0)

### The Foreign Future Protocol (Task 14)

Task 14 is intentionally underspecified because the async foreign trait protocol varies by UniFFI version. The implementer must discover the exact protocol from `nm` output and potentially from UniFFI source code. If it requires infrastructure changes to uniffi-napi itself (new struct field types, new callback patterns), document those as separate follow-up tasks.
