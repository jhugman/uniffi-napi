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

test('callback: invoked from another thread dispatches to event loop', async () => {
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
});
