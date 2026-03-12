import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import lib from '../lib.js';
const { UniffiNativeModule, FfiType } = lib;

const LIB_PATH = join(import.meta.dirname, '..', 'test_lib', 'target', 'debug',
  process.platform === 'darwin' ? 'libuniffi_napi_test_lib.dylib' : 'libuniffi_napi_test_lib.so'
);

const SYMBOLS = {
  rustbufferAlloc: 'uniffi_test_rustbuffer_alloc',
  rustbufferFree: 'uniffi_test_rustbuffer_free',
  rustbufferFromBytes: 'uniffi_test_rustbuffer_from_bytes',
};

function openLib() {
  return UniffiNativeModule.open(LIB_PATH);
}

test('callback: same-thread invocation', () => {
  const lib = openLib();
  const nm = lib.register({
    symbols: SYMBOLS,
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
});

test('VTable: register struct with callbacks, call through', () => {
  const lib = openLib();
  const nm = lib.register({
    symbols: SYMBOLS,
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
});

test('VTable: callback invoked from another thread returns value', async () => {
  const lib = openLib();
  const nm = lib.register({
    symbols: SYMBOLS,
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
      uniffi_test_fn_use_vtable_from_thread: {
        args: [FfiType.UInt64],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
      uniffi_test_fn_is_thread_done: {
        args: [],
        ret: FfiType.Int8,
        hasRustCallStatus: true,
      },
      uniffi_test_fn_get_thread_result: {
        args: [],
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

  // Fire off the cross-thread VTable call (returns immediately)
  const status2 = { code: 0 };
  nm.uniffi_test_fn_use_vtable_from_thread(7n, status2);
  assert.strictEqual(status2.code, 0);

  // Yield to event loop so TSF callback can fire, then poll for completion
  await new Promise((resolve, reject) => {
    let attempts = 0;
    const poll = () => {
      attempts++;
      const pollStatus = { code: 0 };
      const done = nm.uniffi_test_fn_is_thread_done(pollStatus);
      if (done === 1) {
        resolve();
      } else if (attempts > 100) {
        reject(new Error('Timed out waiting for cross-thread VTable callback'));
      } else {
        setImmediate(poll);
      }
    };
    setImmediate(poll);
  });

  // Check the result
  const status3 = { code: 0 };
  const result = nm.uniffi_test_fn_get_thread_result(status3);
  assert.strictEqual(status3.code, 0);
  assert.strictEqual(result, 70); // 7 * 10
});

test('VTable: non-blocking callback invoked from another thread (fire-and-forget)', async () => {
  const lib = openLib();
  let notifiedHandle = null;

  const nm = lib.register({
    symbols: SYMBOLS,
    structs: {
      NotifyVTable: [
        { name: 'notify', type: FfiType.Callback('vtable_notify') },
      ],
    },
    callbacks: {
      vtable_notify: {
        args: [FfiType.UInt64],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
    },
    functions: {
      uniffi_test_fn_init_notify_vtable: {
        args: [FfiType.Reference(FfiType.Struct('NotifyVTable'))],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
      uniffi_test_fn_notify_from_thread: {
        args: [FfiType.UInt64],
        ret: FfiType.Void,
        hasRustCallStatus: true,
      },
    },
  });

  const status1 = { code: 0 };
  nm.uniffi_test_fn_init_notify_vtable({
    notify: (handle) => {
      notifiedHandle = handle;
    },
  }, status1);
  assert.strictEqual(status1.code, 0);

  // Fire off the non-blocking cross-thread call
  const status2 = { code: 0 };
  nm.uniffi_test_fn_notify_from_thread(42n, status2);
  assert.strictEqual(status2.code, 0);

  // Poll on the JS-side effect directly (not the Rust-side NOTIFY_DONE flag).
  await new Promise((resolve, reject) => {
    let attempts = 0;
    const poll = () => {
      attempts++;
      if (notifiedHandle !== null) {
        resolve();
      } else if (attempts > 100) {
        reject(new Error('Timed out waiting for non-blocking VTable callback'));
      } else {
        setImmediate(poll);
      }
    };
    setImmediate(poll);
  });

  assert.strictEqual(notifiedHandle, 42n);
});
