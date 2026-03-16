import { test } from "node:test";
import assert from "node:assert";
import { join } from "node:path";
import lib from "../lib.js";
const { UniffiNativeModule, FfiType } = lib;
import {
  lowerString,
  liftString,
  liftArithmeticError,
} from "./helpers/converters.mjs";
import { continuationCallback, uniffiRustCallAsync } from "./helpers/async.mjs";

const LIB_PATH = join(
  import.meta.dirname,
  "..",
  "fixtures",
  "uniffi-fixture-simple",
  "target",
  "debug",
  process.platform === "darwin"
    ? "libuniffi_fixture_simple.dylib"
    : "libuniffi_fixture_simple.so",
);

const CRATE = "uniffi_fixture_simple";

const SYMBOLS = {
  rustbufferAlloc: `ffi_${CRATE}_rustbuffer_alloc`,
  rustbufferFree: `ffi_${CRATE}_rustbuffer_free`,
  rustbufferFromBytes: `ffi_${CRATE}_rustbuffer_from_bytes`,
};

function openAndRegister(
  extraFunctions = {},
  extraCallbacks = {},
  extraStructs = {},
) {
  const mod = UniffiNativeModule.open(LIB_PATH);
  return mod.register({
    symbols: SYMBOLS,
    structs: extraStructs,
    callbacks: extraCallbacks,
    functions: extraFunctions,
  });
}

test('fixture: greet("World") = "Hello, World!" (sync string)', () => {
  const nm = openAndRegister({
    [`uniffi_${CRATE}_fn_func_greet`]: {
      args: [FfiType.RustBuffer],
      ret: FfiType.RustBuffer,
      hasRustCallStatus: true,
    },
  });

  const status = { code: 0 };
  const result = nm[`uniffi_${CRATE}_fn_func_greet`](
    lowerString("World"),
    status,
  );
  assert.strictEqual(status.code, 0);
  assert.strictEqual(liftString(result), "Hello, World!");
});

test("fixture: add(3, 4) = 7 (sync scalar)", () => {
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

test("fixture: divide(1.0, 0.0) returns error (sync error path)", () => {
  const nm = openAndRegister({
    [`uniffi_${CRATE}_fn_func_divide`]: {
      args: [FfiType.Float64, FfiType.Float64],
      ret: FfiType.Float64,
      hasRustCallStatus: true,
    },
  });

  const status = { code: 0 };
  nm[`uniffi_${CRATE}_fn_func_divide`](1.0, 0.0, status);
  assert.notStrictEqual(status.code, 0, "Expected non-zero error code");
  assert.ok(status.errorBuf instanceof Uint8Array, "Expected errorBuf");

  const error = liftArithmeticError(status.errorBuf);
  assert.strictEqual(error.variant, 1); // DivisionByZero
  assert.ok(error.reason.includes("cannot divide by zero"));
});

test("fixture: divide(10.0, 2.0) = 5.0 (sync success path)", () => {
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

test("fixture: async_add(3, 4) = 7 (async scalar)", async () => {
  const nm = openAndRegister(
    {
      [`uniffi_${CRATE}_fn_func_async_add`]: {
        args: [FfiType.UInt32, FfiType.UInt32],
        ret: FfiType.Handle,
        hasRustCallStatus: true,
      },
      [`ffi_${CRATE}_rust_future_poll_u32`]: {
        args: [
          FfiType.Handle,
          FfiType.Callback("rust_future_continuation"),
          FfiType.UInt64,
        ],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      [`ffi_${CRATE}_rust_future_complete_u32`]: {
        args: [FfiType.Handle],
        ret: FfiType.UInt32,
        hasRustCallStatus: true,
      },
      [`ffi_${CRATE}_rust_future_free_u32`]: {
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
    },
  );

  const result = await uniffiRustCallAsync(nm, {
    rustFutureFunc: () => {
      const status = { code: 0 };
      return nm[`uniffi_${CRATE}_fn_func_async_add`](3, 4, status);
    },
    pollFunc: `ffi_${CRATE}_rust_future_poll_u32`,
    completeFunc: `ffi_${CRATE}_rust_future_complete_u32`,
    freeFunc: `ffi_${CRATE}_rust_future_free_u32`,
  });

  assert.strictEqual(result, 7);
});

// Shared Calculator VTable registration definitions.
//
// The UniFFI 0.31 VTable struct for Calculator has this C layout:
//   { uniffi_free, uniffi_clone, add, concatenate }
// Each method callback uses the "out-return" convention: the return value is
// written through an out-pointer argument, and the C function itself returns void.
const CALCULATOR_CALLBACKS = {
  callback_calculator_free: {
    args: [FfiType.UInt64],
    ret: FfiType.Void,
    hasRustCallStatus: false,
  },
  callback_calculator_clone: {
    args: [FfiType.UInt64],
    ret: FfiType.UInt64,
    hasRustCallStatus: false,
  },
  callback_calculator_add: {
    args: [FfiType.UInt64, FfiType.UInt32, FfiType.UInt32],
    ret: FfiType.UInt32,
    hasRustCallStatus: true,
    outReturn: true,
  },
  callback_calculator_concatenate: {
    args: [FfiType.UInt64, FfiType.RustBuffer, FfiType.RustBuffer],
    ret: FfiType.RustBuffer,
    hasRustCallStatus: true,
    outReturn: true,
  },
};

const CALCULATOR_STRUCT = {
  VTable_Calculator: [
    { name: "uniffi_free", type: FfiType.Callback("callback_calculator_free") },
    {
      name: "uniffi_clone",
      type: FfiType.Callback("callback_calculator_clone"),
    },
    { name: "add", type: FfiType.Callback("callback_calculator_add") },
    {
      name: "concatenate",
      type: FfiType.Callback("callback_calculator_concatenate"),
    },
  ],
};

const CALCULATOR_VTABLE_JS = {
  uniffi_free: (handle) => {},
  uniffi_clone: (handle) => handle,
  add: (handle, a, b, callStatus) => {
    callStatus.code = 0;
    return a + b;
  },
  concatenate: (handle, aBuf, bBuf, callStatus) => {
    callStatus.code = 0;
    return lowerString(liftString(aBuf) + liftString(bBuf));
  },
};

test("fixture: Calculator.add via VTable (sync callback, scalar)", () => {
  const nm = openAndRegister(
    {
      [`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]: {
        args: [FfiType.Reference(FfiType.Struct("VTable_Calculator"))],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      [`uniffi_${CRATE}_fn_func_use_calculator`]: {
        args: [FfiType.UInt64, FfiType.UInt32, FfiType.UInt32],
        ret: FfiType.UInt32,
        hasRustCallStatus: true,
      },
    },
    CALCULATOR_CALLBACKS,
    CALCULATOR_STRUCT,
  );

  // Register VTable
  nm[`uniffi_${CRATE}_fn_init_callback_vtable_calculator`](
    CALCULATOR_VTABLE_JS,
  );

  // Call use_calculator(calc_handle, 3, 4) => should invoke add(3,4) => 7
  const status2 = { code: 0 };
  const result = nm[`uniffi_${CRATE}_fn_func_use_calculator`](
    1n,
    3,
    4,
    status2,
  );
  assert.strictEqual(status2.code, 0);
  assert.strictEqual(result, 7);
});

test("fixture: Calculator.concatenate via VTable (sync callback, string)", () => {
  const nm = openAndRegister(
    {
      [`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]: {
        args: [FfiType.Reference(FfiType.Struct("VTable_Calculator"))],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      [`uniffi_${CRATE}_fn_func_use_calculator_strings`]: {
        args: [FfiType.UInt64, FfiType.RustBuffer, FfiType.RustBuffer],
        ret: FfiType.RustBuffer,
        hasRustCallStatus: true,
      },
    },
    CALCULATOR_CALLBACKS,
    CALCULATOR_STRUCT,
  );

  // Register VTable
  nm[`uniffi_${CRATE}_fn_init_callback_vtable_calculator`](
    CALCULATOR_VTABLE_JS,
  );

  // Call use_calculator_strings(calc_handle, "Hello, ", "World!") => "Hello, World!"
  const status2 = { code: 0 };
  const result = nm[`uniffi_${CRATE}_fn_func_use_calculator_strings`](
    1n,
    lowerString("Hello, "),
    lowerString("World!"),
    status2,
  );
  assert.strictEqual(status2.code, 0);
  assert.strictEqual(liftString(result), "Hello, World!");
});

test("fixture: Calculator.add from background thread (async + cross-thread VTable)", async () => {
  const nm = openAndRegister(
    {
      [`uniffi_${CRATE}_fn_init_callback_vtable_calculator`]: {
        args: [FfiType.Reference(FfiType.Struct("VTable_Calculator"))],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      [`uniffi_${CRATE}_fn_func_use_calculator_from_thread`]: {
        args: [FfiType.UInt64, FfiType.UInt32, FfiType.UInt32],
        ret: FfiType.Handle,
        hasRustCallStatus: true,
      },
      [`ffi_${CRATE}_rust_future_poll_u32`]: {
        args: [
          FfiType.Handle,
          FfiType.Callback("rust_future_continuation"),
          FfiType.UInt64,
        ],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      [`ffi_${CRATE}_rust_future_complete_u32`]: {
        args: [FfiType.Handle],
        ret: FfiType.UInt32,
        hasRustCallStatus: true,
      },
      [`ffi_${CRATE}_rust_future_free_u32`]: {
        args: [FfiType.Handle],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
    },
    {
      ...CALCULATOR_CALLBACKS,
      rust_future_continuation: {
        args: [FfiType.UInt64, FfiType.Int8],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
    },
    CALCULATOR_STRUCT,
  );

  // Register the Calculator VTable so Rust can call back into JS
  nm[`uniffi_${CRATE}_fn_init_callback_vtable_calculator`](
    CALCULATOR_VTABLE_JS,
  );

  // Call use_calculator_from_thread(calc_handle, 3, 4) — async, spawns a blocking
  // thread that invokes calc.add() via the VTable, dispatched back to JS via TSF.
  const result = await uniffiRustCallAsync(nm, {
    rustFutureFunc: () => {
      const status = { code: 0 };
      return nm[`uniffi_${CRATE}_fn_func_use_calculator_from_thread`](
        1n,
        3,
        4,
        status,
      );
    },
    pollFunc: `ffi_${CRATE}_rust_future_poll_u32`,
    completeFunc: `ffi_${CRATE}_rust_future_complete_u32`,
    freeFunc: `ffi_${CRATE}_rust_future_free_u32`,
  });

  assert.strictEqual(result, 7);
});

test('fixture: async_greet("World") = "Hello, World!" (async string)', async () => {
  const nm = openAndRegister(
    {
      [`uniffi_${CRATE}_fn_func_async_greet`]: {
        args: [FfiType.RustBuffer],
        ret: FfiType.Handle,
        hasRustCallStatus: true,
      },
      [`ffi_${CRATE}_rust_future_poll_rust_buffer`]: {
        args: [
          FfiType.Handle,
          FfiType.Callback("rust_future_continuation"),
          FfiType.UInt64,
        ],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      [`ffi_${CRATE}_rust_future_complete_rust_buffer`]: {
        args: [FfiType.Handle],
        ret: FfiType.RustBuffer,
        hasRustCallStatus: true,
      },
      [`ffi_${CRATE}_rust_future_free_rust_buffer`]: {
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
    },
  );

  const result = await uniffiRustCallAsync(nm, {
    rustFutureFunc: () => {
      const status = { code: 0 };
      return nm[`uniffi_${CRATE}_fn_func_async_greet`](
        lowerString("World"),
        status,
      );
    },
    pollFunc: `ffi_${CRATE}_rust_future_poll_rust_buffer`,
    completeFunc: `ffi_${CRATE}_rust_future_complete_rust_buffer`,
    freeFunc: `ffi_${CRATE}_rust_future_free_rust_buffer`,
    liftFunc: liftString,
  });

  assert.strictEqual(result, "Hello, World!");
});

test('fixture: register with struct-by-value does not crash', () => {
  const mod = UniffiNativeModule.open(LIB_PATH);
  assert.doesNotThrow(() => {
    mod.register({
      symbols: SYMBOLS,
      functions: {},
      callbacks: {
        test_cb: {
          args: [FfiType.Struct('TestResult')],
          ret: FfiType.Void,
          hasRustCallStatus: false,
        },
      },
      structs: {
        TestResult: [
          { name: 'value', type: FfiType.UInt32 },
          { name: 'code', type: FfiType.Int8 },
        ],
      },
    });
  });
});

// TODO: AsyncFetcher foreign trait test (async callback)
//
// This test exercises UniFFI's "foreign future" protocol for async foreign traits.
// The AsyncFetcher trait has one async method:
//
//   #[uniffi::export(with_foreign)]
//   #[async_trait]
//   pub trait AsyncFetcher: Send + Sync {
//       async fn fetch(&self, input: String) -> String;
//   }
//
// The generated VTable struct for AsyncFetcher has this C layout:
//
//   struct VTable_AsyncFetcher {
//       uniffi_free:  extern "C" fn(handle: u64),
//       uniffi_clone: extern "C" fn(handle: u64) -> u64,
//       fetch:        extern "C" fn(
//                         handle: u64,
//                         input: RustBuffer,
//                         complete_callback: ForeignFutureCallback<RustBuffer>,
//                         complete_callback_data: u64,
//                         out_dropped_callback: &mut ForeignFutureDroppedCallbackStruct,
//                     ),
//   }
//
// The foreign future protocol works as follows:
// 1. Rust calls `vtable.fetch(handle, input, complete_cb, complete_data, &mut dropped_cb)`
// 2. JS performs the async work and, when done, calls `complete_cb(complete_data, result)`
//    where `result` is a ForeignFutureResult<RustBuffer>:
//      struct ForeignFutureResult<RustBuffer> {
//          return_value: RustBuffer,    // the serialized return value
//          call_status: RustCallStatus, // {code: i8, error_buf: RustBuffer}
//      }
// 3. Optionally, JS writes a {callback_data: u64, callback: fn(u64)} into out_dropped_callback
//    so Rust can notify JS if the future is cancelled.
//
// Infrastructure gaps that prevent this test from working today:
//
// 1. ForeignFutureCallback: The `complete_callback` parameter is a function pointer
//    (extern "C" fn) that Rust passes INTO the JS callback. The JS side needs to be
//    able to *call* this function pointer later. Currently, callback args of type
//    "Callback" create libffi closures (JS->C direction), but here we need the
//    reverse: receive a C function pointer and call it from JS with a struct argument.
//    This requires a new mechanism — e.g., a way to invoke a raw C function pointer
//    from JS, passing a ForeignFutureResult struct by value.
//
// 2. ForeignFutureResult struct by value: The complete_callback takes a
//    ForeignFutureResult<RustBuffer> as a by-value struct argument (not a pointer).
//    This struct contains {RustBuffer, RustCallStatus} which is
//    {RustBuffer, {i8, RustBuffer}}. Passing structs by value to C function pointers
//    requires libffi struct type construction, which is not yet exposed to JS.
//
// 3. ForeignFutureDroppedCallbackStruct out-pointer: The `out_dropped_callback`
//    parameter is a mutable reference to a struct {u64, fn(u64)}. The JS callback
//    needs to write fields (including a function pointer) into this struct through
//    the pointer. The current trampoline reads args but does not support writing
//    to out-pointer struct fields that contain function pointers.
//
// To implement this, uniffi-napi would need:
// - A "ForeignFutureCallback" FfiType that represents a C function pointer received
//   as an argument, which JS can later invoke via a helper (e.g., nm.callFnPtr()).
// - Support for constructing and passing C structs by value when calling function
//   pointers (for ForeignFutureResult).
// - Support for writing to mutable struct out-pointers from JS (for the dropped
//   callback struct).
//
test(
  "fixture: AsyncFetcher (async foreign trait)",
  {
    skip: "Requires foreign future protocol support in uniffi-napi — see TODO comments above",
  },
  () => {
    // When the infrastructure is ready, the test would look roughly like:
    //
    // const nm = openAndRegister(
    //   {
    //     [`uniffi_${CRATE}_fn_init_callback_vtable_asyncfetcher`]: {
    //       args: [FfiType.Reference(FfiType.Struct('VTable_AsyncFetcher'))],
    //       ret: FfiType.Void,
    //       hasRustCallStatus: false,
    //     },
    //     [`uniffi_${CRATE}_fn_func_use_async_fetcher`]: {
    //       args: [FfiType.UInt64, FfiType.RustBuffer],
    //       ret: FfiType.Handle,
    //       hasRustCallStatus: true,
    //     },
    //     // ... rust_future_poll/complete/free for RustBuffer ...
    //   },
    //   { /* callback defs for fetch, free, clone */ },
    //   { /* struct defs for VTable_AsyncFetcher, ForeignFutureResult */ },
    // );
    //
    // // Register VTable where fetch callback:
    // // 1. Receives (handle, inputBuf, completeCbPtr, completeCbData, droppedCbOutPtr)
    // // 2. Lifts inputBuf to get the input string
    // // 3. Computes the result (e.g., "fetched: " + input)
    // // 4. Calls completeCbPtr(completeCbData, {return_value: lowerString(result), call_status: {code: 0}})
    // nm[`uniffi_${CRATE}_fn_init_callback_vtable_asyncfetcher`](vtableImpl);
    //
    // const result = await uniffiRustCallAsync(nm, {
    //   rustFutureFunc: () => nm[`uniffi_${CRATE}_fn_func_use_async_fetcher`](1n, lowerString('hello'), status),
    //   pollFunc: ..., completeFunc: ..., freeFunc: ...,
    //   liftFunc: liftString,
    // });
    //
    // assert.strictEqual(result, 'fetched: hello');
  },
);
