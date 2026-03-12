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

test('register and call i32 add function', () => {
  const lib = openLib();
  const nm = lib.register({
    symbols: SYMBOLS,
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
});

test('register and call i8 negate function', () => {
  const lib = openLib();
  const nm = lib.register({
    symbols: SYMBOLS,
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
});

test('register and call u64 handle function', () => {
  const lib = openLib();
  const nm = lib.register({
    symbols: SYMBOLS,
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
});

test('register and call void function', () => {
  const lib = openLib();
  const nm = lib.register({
    symbols: SYMBOLS,
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
});

test('register and call f64 double function', () => {
  const lib = openLib();
  const nm = lib.register({
    symbols: SYMBOLS,
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
});
