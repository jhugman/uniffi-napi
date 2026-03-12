# uniffi-napi: UniFFI-specialized napi-rs native module for Node.js

## Problem

uniffi-bindgen-node uses ffi-rs as its bridge between Node.js and native Rust libraries. ffi-rs is a general-purpose FFI module that has several limitations that make completing uniffi-bindgen-node intractable:

- No `Int8` type support (workarounds needed for RustCallStatus.code and booleans)
- BigInt precision loss for `uint64_t` (object handles and async callback data truncated to JS number)
- Clunky struct-by-value passing (requires `ffiTypeTag` hacks for RustBuffer and RustCallStatus)
- Fire-and-forget callbacks on the main thread only (no threading support for foreign traits, async polling, or async callbacks)
- Poor output parameter ergonomics (RustCallStatus as `&mut` pointer)

These limitations block: callback interfaces, foreign traits, async functions, and async callbacks.

## Solution

Build `uniffi-napi`, a single napi-rs native addon specialized for the UniFFI calling convention. It replaces ffi-rs as the FFI bridge while the shared TypeScript runtime from uniffi-bindgen-react-native continues to handle RustBuffer serialization, FfiConverters, error checking, and async orchestration.

## Architecture

```
  Generated TypeScript          Shared TS Runtime (ubrn)
  +--------------+              +---------------------+
  | *-ffi.ts     |--exports-->  | UniffiRustCaller     |
  |  register()  |  NativeModule| uniffiRustCallAsync   |
  |  structs,    |  Interface   | FfiConverters, etc.   |
  |  callbacks,  |              +---------------------+
  |  functions   |
  +------+-------+
         | calls into
  +------v----------------------------------------+
  |  uniffi-napi  (napi-rs cdylib)                |
  |                                               |
  |  open()     -> dlopen target library          |
  |  register() -> build CIFs, trampolines        |
  |  call       -> marshal JS<->C, ffi_call       |
  |                                               |
  |  Internals:                                   |
  |  +-- dlopen (symbol lookup)                   |
  |  +-- libffi (calling convention)              |
  |  +-- RustBuffer <-> Uint8Array                |
  |  +-- RustCallStatus <-> JS object             |
  |  +-- ThreadsafeFunction (callbacks)           |
  |  +-- main-thread detection                    |
  +-----------------------------------------------+
         | ffi_call / trampolines
  +------v-------+
  | Target Rust  |
  | .dylib/.so   |
  +--------------+
```

### How the three layers relate

- **uniffi-napi** provides `open()` and `register()`. It handles dlopen, libffi CIF preparation, JS-to-C marshaling, C-to-JS marshaling, and threadsafe callback dispatch.
- **Generated `*-ffi.ts`** calls `open()` with the library path, then `register()` with per-crate RustBuffer symbols, structs, callbacks, and functions from `ci.ffi_definitions()`. It exports the resulting object as the `NativeModuleInterface`. In a megazord setup, multiple crates share one `open()` call.
- **Shared ubrn runtime** owns `UniffiRustCaller` (checks RustCallStatus), `uniffiRustCallAsync` (polls Rust futures), `FfiConverter*` (serializes types into RustBuffer), and `callbacks.ts` (foreign trait interface helpers). It calls through the `NativeModuleInterface` without knowing which backend is behind it.

## API

### open()

```typescript
import { UniffiNativeModule } from 'uniffi-napi';

const lib = UniffiNativeModule.open('/path/to/libfoo.dylib');
```

`open()` is a `#[napi]` function that:
1. Calls `dlopen` on the target library
2. Returns a `UniffiNativeModule` handle

In a megazord (multi-crate) setup, `open()` is called once and the handle is shared across multiple `register()` calls.

### register()

```typescript
const nativeModule = lib.register({
  symbols: {
    rustbufferAlloc: 'uniffi_foo_rustbuffer_alloc',
    rustbufferFree: 'uniffi_foo_rustbuffer_free',
    rustbufferFromBytes: 'uniffi_foo_rustbuffer_from_bytes',
  },
  structs: {
    UniffiRustCallStatus: [
      { name: 'code', type: FfiType.Int8 },
      { name: 'error_buf', type: FfiType.RustBuffer },
    ],
    VTable_MyTrait: [
      { name: 'do_thing', type: FfiType.Callback('callback_my_trait_do_thing') },
      { name: 'uniffi_free', type: FfiType.Callback('callback_free') },
    ],
  },
  callbacks: {
    callback_my_trait_do_thing: {
      args: [FfiType.Handle, FfiType.Int32],
      ret: FfiType.Void,
      hasRustCallStatus: true,
    },
    UniffiRustFutureContinuationCallback: {
      args: [FfiType.UInt64, FfiType.Int8],
      ret: FfiType.Void,
      hasRustCallStatus: false,
    },
  },
  functions: {
    uniffi_foo_fn_bar: {
      args: [FfiType.Int32, FfiType.RustBuffer],
      ret: FfiType.Handle,
      hasRustCallStatus: true,
    },
  },
});
```

`register()` is a `#[napi]` method that:
1. Iterates struct definitions, builds libffi type descriptors for each
2. Iterates callback definitions, builds CIFs and allocates trampoline function pointers for each
3. Iterates function definitions, looks up each symbol via dlopen, builds a libffi CIF with the argument/return types
4. Returns a JS object where each function name is a property bound to a Rust closure that captures the CIF and symbol pointer

At call time, there are no string lookups or type resolution — everything is pre-compiled.

### Calling convention

Each registered function becomes a property on the returned object. Functions take **positional arguments** matching their `args` definition. If `hasRustCallStatus` is true, the final argument is always the `{ code, errorBuf? }` object.

```typescript
// For a function registered as:
//   uniffi_foo_fn_bar: { args: [FfiType.Int32, FfiType.RustBuffer], ret: FfiType.Handle, hasRustCallStatus: true }
// The call looks like:
const result: bigint = nativeModule.uniffi_foo_fn_bar(42, someUint8Array, callStatus);
```

The returned object's TypeScript type is effectively:

```typescript
type NativeModuleInterface = Record<string, (...args: any[]) => any>;
```

The generated `*-ffi.ts` casts this to a specific interface with typed signatures per function, matching the pattern in ubrn's `wrapper-ffi.ts`.

### VTable registration

`register()` only receives struct/callback/function *definitions* (schemas). It does not receive JS callback function values. VTable instances with actual JS functions are passed later, when calling VTable init functions (e.g. `uniffi_foo_fn_init_callback_vtable_mytrait`).

When a registered function has a `Struct(name)` argument, and that struct has `Callback(name)` fields, the napi module:

1. Accepts a JS object with properties matching the struct fields
2. For `Callback` fields, wraps each JS function in a ThreadsafeFunction and allocates a C trampoline (via `ffi_closure_alloc`)
3. Builds the C struct with trampoline function pointers
4. Passes the struct (by reference) to the C function

Trampoline lifetimes are tied to the `UniffiNativeModule` instance. They are freed when the module is closed or garbage collected. This is safe because VTable init functions are called once at startup, and Rust holds the function pointers for the lifetime of the component.

### Error handling

- `open()` throws a JS `Error` if `dlopen` fails (e.g. library not found, architecture mismatch)
- `register()` throws if a function symbol is not found in the opened library
- Both propagate the underlying OS/dlopen error message

### FfiType TypeScript representation

`FfiType` is an object with static constants for simple types and factory functions for parameterized types:

```typescript
const FfiType = {
  UInt8: { tag: 'UInt8' },
  Int8: { tag: 'Int8' },
  // ...all scalar types...
  Handle: { tag: 'Handle' },
  RustBuffer: { tag: 'RustBuffer' },
  ForeignBytes: { tag: 'ForeignBytes' },
  RustCallStatus: { tag: 'RustCallStatus' },
  VoidPointer: { tag: 'VoidPointer' },
  Void: { tag: 'Void' },
  // Factory functions for parameterized types:
  Callback: (name: string) => ({ tag: 'Callback', name }),
  Struct: (name: string) => ({ tag: 'Struct', name }),
  Reference: (inner: FfiTypeDesc) => ({ tag: 'Reference', inner }),
  MutReference: (inner: FfiTypeDesc) => ({ tag: 'MutReference', inner }),
} as const;
```

`FfiType.Callback('name')` in a struct field definition references a callback definition from the `callbacks` map in the same `register()` call. The module resolves these cross-references at registration time.

## FfiType Mapping

Based on uniffi_bindgen's `FfiType` enum (uniffi 0.31.x):

| FfiType variant | JS representation | C representation | Notes |
|---|---|---|---|
| `UInt8, Int8` | `number` | `uint8_t, int8_t` | Int8 was missing from ffi-rs |
| `UInt16, Int16` | `number` | `uint16_t, int16_t` | |
| `UInt32, Int32` | `number` | `uint32_t, int32_t` | |
| `UInt64, Int64` | `bigint` | `uint64_t, int64_t` | Always BigInt, no precision loss |
| `Float32, Float64` | `number` | `float, double` | |
| `Handle` | `bigint` | `uint64_t` | Opaque object pointers |
| `RustBuffer(meta)` | `Uint8Array` | struct by value | Module handles alloc/free internally. `meta` carries namespace info; since `register()` binds rustbuffer symbols per-crate, `meta` is not needed at runtime. |
| `ForeignBytes` | `Uint8Array` | struct by value | Borrowed; napi module holds a `napi::Ref` to prevent GC during the call |
| `RustCallStatus` | `{ code, errorBuf? }` | via `MutReference` | Module marshals JS object to/from C struct |
| `Callback(name)` | JS function | C function pointer | Wrapped in ThreadsafeFunction trampoline |
| `Struct(name)` | JS object | C struct | VTables for foreign traits |
| `Reference(inner)` | depends on inner | `const *inner` | |
| `MutReference(inner)` | depends on inner | `*mut inner` | Used for RustCallStatus output param |
| `VoidPointer` | `bigint` | `void*` | |

## RustCallStatus Flow

RustCallStatus is visible to the TypeScript layer. The napi module handles marshaling, not error checking.

1. JS creates `{ code: 0 }` (via `UniffiRustCaller.createCallStatus()`)
2. JS passes it as the last argument to a registered function
3. Rust (napi) side: reads `code` from the JS object, allocates a C `RustCallStatus` on the stack with that value and an empty `error_buf`
4. Passes `&mut RustCallStatus` to the C function via libffi
5. After `ffi_call` returns: reads the mutated C struct and writes `code` back into the JS object
6. If `error_buf.data` is non-null: copies `error_buf.len` bytes into a new `Uint8Array`, sets it as `errorBuf` on the JS object, then calls `rustbuffer_free` on the error buffer
7. JS side: `UniffiRustCaller` checks `code` and lifts errors — unchanged from current ubrn runtime

## RustBuffer Flow

RustBuffer is opaque to TypeScript. JS sees `Uint8Array` on both sides.

**Lowering (JS to Rust — passing arguments):**
1. JS passes a `Uint8Array` (already serialized by FfiConverter)
2. Rust side: calls the library's `rustbuffer_from_bytes` with a `ForeignBytes` pointing to the Uint8Array data
3. Gets back a C `RustBuffer` struct
4. Passes the struct by value to `ffi_call`

**Lifting (Rust to JS — return values):**
1. `ffi_call` returns a `RustBuffer` struct
2. Rust side: reads `len` bytes from `data` pointer into a new `Uint8Array`
3. Calls the library's `rustbuffer_free` to release the C allocation
4. Returns the `Uint8Array` to JS

**RustCallStatus error_buf:**
- Same lifting path — if `code != 0` and `error_buf.data` is non-null, copy bytes into `Uint8Array` and set on JS object

## Threading & Callbacks

### Main thread detection

During `#[napi::module_init]` (which napi-rs guarantees runs on the Node.js main thread), capture `std::thread::current().id()` as `main_thread_id`. Every callback trampoline checks `current_thread == main_thread_id` to decide dispatch strategy.

### Callback dispatch

UniFFI does not support passing closures across the FFI. All user-facing callbacks are delivered through **VTable structs** (foreign trait vtables). The only non-VTable function pointers are internal ones generated by UniFFI's scaffolding (e.g. async continuation callbacks).

**Same thread (main thread):**
- Call the JS function directly, synchronously
- Return values and RustCallStatus writeback work normally
- This is the fast path for synchronous trait callbacks invoked from the main thread

**Different thread — implicit blocking/non-blocking rule:**

The dispatch mode is determined by the callback's own properties — no explicit flag needed:

| Condition | Mode | Behavior |
|-----------|------|----------|
| `ret != Void` or `hasRustCallStatus: true` | **Blocking** | Calling thread blocks until JS completes; return value and RustCallStatus sent back via `mpsc::sync_channel` |
| `ret == Void` and `hasRustCallStatus: false` | **Non-blocking** | Fire-and-forget via `ThreadsafeFunction` (NonBlocking); calling thread does not wait |

User callbacks (foreign trait methods) always return a value or have RustCallStatus, so they use blocking dispatch. Internal scaffolding callbacks (async continuations, free) have void return and no RustCallStatus, so they fire-and-forget.

**Implementation:** Each VTable callback trampoline has an associated `ThreadsafeFunction` created at VTable build time. The TSF is created from a no-op JS function to avoid napi-rs double-invocation (napi-rs auto-calls the TSF's base function with the callback's return values). The real JS function is called manually in the handler via `fn_ref`. The TSF is `unref()`'d so it doesn't keep the event loop alive.

For blocking dispatch, the calling thread packs args into a `VTableCallRequest` (with a `SyncSender<VTableCallResponse>`), dispatches via TSF (Blocking mode), and blocks on `rx.recv()`. The JS-thread handler calls the JS function, marshals the return value into `RawCallbackArg`, reads back RustCallStatus, and sends the response through the channel.

**Deadlock risk with blocking callbacks:** If a foreign trait method (dispatched to JS via blocking ThreadsafeFunction) itself calls back into Rust synchronously, and that Rust call invokes another blocking callback, the Rust thread is already blocked and cannot proceed. This is inherent to single-threaded JS runtimes. Mitigation: foreign trait methods invoked from non-main threads must not re-enter Rust synchronously from within the JS callback. This is the same constraint that exists in other UniFFI bindings (e.g. Swift). In practice, most foreign trait methods are simple and do not call back into Rust.

**Test deadlock avoidance:** Tests that exercise cross-thread VTable callbacks must not `.join()` on the spawned thread from the main thread (classic deadlock: spawned thread dispatches to JS main thread, but main thread is blocked in `.join()`). Instead, use a fire-and-forget pattern: spawn the thread, return immediately, let JS yield to the event loop via `setImmediate` polling, then check results.

### Async future polling

The `UniffiRustFutureContinuationCallback` fires from whatever thread the Rust executor uses. The trampoline dispatches to the Node.js event loop via ThreadsafeFunction (non-blocking). This resolves the Promise in `pollRust()`, which schedules the next `rust_future_poll` call — naturally non-reentrant, matching the requirement from uniffi-rs.

The `uniffiRustCallAsync` orchestration in the shared ubrn runtime works unchanged.

### Foreign trait callbacks

VTable structs contain function pointers for each trait method. VTable *instances* are constructed at call time (when the generated code calls a VTable init function like `uniffi_foo_fn_init_callback_vtable_mytrait`), not at `register()` time. See the "VTable registration" section above for the full flow.

Each VTable callback trampoline stores:
- `fn_ref`: persistent `napi_ref` to the JS function (GC-safe)
- `arg_types`, `ret_type`, `has_rust_call_status`: callback signature
- `tsfn`: `ThreadsafeFunction<VTableCallRequest>` for cross-thread dispatch

Trampolines are intentionally leaked (`Box::into_raw` + `mem::forget`) for module lifetime. The TSF is `unref()`'d so it doesn't keep the event loop alive. When `UniffiNativeModule` is garbage collected, the dlopen handle closes; leaked trampolines become stale but are never called again (Rust side is also torn down).

### Async foreign callbacks

**Deferred:** The `ForeignFuture` mechanism for async foreign traits involves specific C structs (`ForeignFuture`, `ForeignFutureCallback`) and calling conventions that interact with the Rust executor. This is the most complex callback scenario and will be designed in detail as a follow-up once sync foreign traits are working. The architecture (ThreadsafeFunction + main-thread detection) supports it, but the precise marshaling flow needs its own specification.

## What This Unlocks for uniffi-bindgen-node

| Feature | Status with ffi-rs | Status with uniffi-napi |
|---|---|---|
| Sync functions | Working | Working |
| Records, enums | Working | Working |
| Objects | Working (BigInt workarounds) | Working (clean) |
| Async functions | Partial (precision loss, threading hacks) | Working |
| Callback interfaces | Not possible | Working |
| Foreign traits | Not possible | Working |
| Async callbacks | Not possible | Working |

## Worked Example: Sync Function Call

End-to-end for `uniffi_foo_fn_bar(x: i32, buf: RustBuffer) -> Handle` with RustCallStatus:

```
JS:  const status = { code: 0 };
     const result = nativeModule.uniffi_foo_fn_bar(42, myUint8Array, status);

  → napi closure fires (CIF + symbol pointer pre-captured)
  → reads JS arg 0 (number 42) → C int32_t
  → reads JS arg 1 (Uint8Array) → calls rustbuffer_from_bytes → C RustBuffer struct
  → reads JS arg 2 (status obj) → allocates C RustCallStatus { code: 0, error_buf: empty }
  → ffi_call(symbol, [42, rustbuffer, &mut call_status]) → returns uint64_t handle
  → writes call_status.code back to status.code
  → if error_buf non-null: copies to Uint8Array, sets status.errorBuf, frees error_buf
  → returns handle as BigInt to JS

JS:  uniffiCheckCallStatus(status, ...);
     return result; // bigint
```

## Worked Example: Async Continuation Callback

When Rust calls the continuation callback from a worker thread:

```
Rust thread:  continuation_callback(handle: u64, poll_result: i8)
  → C trampoline fires
  → checks std::thread::current().id() != main_thread_id
  → calls napi_call_threadsafe_function (non-blocking)
  → returns immediately (Rust thread not blocked)

Node event loop (next tick):
  → JS callback fires: (handle: bigint, pollResult: number) => void
  → resolves the Promise in pollRust()
  → uniffiRustCallAsync loop continues with next poll
```

## Library Lifecycle

`UniffiNativeModule` holds:
- The `dlopen` library handle
- Pre-compiled libffi CIFs for all registered functions
- Trampoline allocations (`ffi_closure_alloc`) for callbacks
- ThreadsafeFunction references for active callbacks

Cleanup happens when the `UniffiNativeModule` is garbage collected via napi-rs `Drop` implementation, which closes the dlopen handle. There is no explicit `close()` method — registered functions hold raw symbol pointers into the library, so the library must outlive all registered functions. GC naturally enforces this since both are held in the same module scope.

## Breaking Changes from ffi-rs

Switching from ffi-rs to uniffi-napi requires template changes in uniffi-bindgen-node:
- Object handles change from `JsExternal` to `bigint`
- `RustBuffer` changes from manual `createPointer`/`restorePointer`/`freePointer` juggling to plain `Uint8Array`
- `RustCallStatus` changes from `JsExternal` pointer manipulation to a plain JS object
- The entire `sys.ts` template is replaced with a much simpler `*-ffi.ts` that calls `open()` + `register()`
- `node.ts` template simplifies: no more `ffi-rs` imports, no `DataType.*`, no `ffiTypeTag` hacks

## Implementation Risks

### libffi struct-by-value passing across platforms

Passing `RustBuffer` (a struct with `capacity: i64, len: i64, data: *mut u8`) by value through libffi is the highest technical risk. Struct-by-value calling conventions vary across platforms and ABIs (especially ARM64 where small structs may go in registers). libffi handles this, but the `ffi_type` descriptors must be built correctly for each struct. This requires thorough cross-platform testing, particularly on `linux-arm64-gnu` and `win32-x64-msvc`.

### libffi-sys compilation on Windows

The `libffi-sys` crate requires a C toolchain (and sometimes autotools) to build libffi from source. On Windows MSVC targets this has historically been problematic. Since this ships as a pre-built npm package, the complexity is contained to CI, but it should be validated early. Mitigation: vendored or pre-built libffi binaries if the build proves unreliable.

## Non-Goals

- **Hot-reload of native libraries**: not supported; library lifetime is tied to GC of the `UniffiNativeModule`
- **Multiple simultaneous libraries**: supported (call `open()` multiple times), but each returns a separate `UniffiNativeModule`
- **npm package structure**: follows standard napi-rs `@scope/package-platform` optional dependencies pattern for pre-built binaries

## Implementation Constraints

- **Single pre-built npm package**: ships platform binaries, no per-library Rust build step needed
- **Dependencies**: napi-rs, dlopen crate, libffi/libffi-sys
- **Platform targets**: at minimum darwin-x64, darwin-arm64, linux-x64-gnu, linux-arm64-gnu, win32-x64-msvc
- **Node.js version**: N-API version 6+ (Node 10.20+) for ThreadsafeFunction support

## Future Optimization (not in scope)

Direction 2 from the design discussion: per-library codegen that generates typed napi-rs Rust wrappers, eliminating libffi entirely. This would be a performance optimization if libffi overhead becomes a concern.

