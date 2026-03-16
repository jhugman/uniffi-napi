import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import lib from '../lib.js';
const { UniffiNativeModule, FfiType } = lib;
import { lowerString, liftString, liftArithmeticError } from './helpers/converters.mjs';

const LIB_PATH = join(import.meta.dirname, '..', 'fixtures', 'uniffi-fixture-simple',
  'target', 'debug',
  process.platform === 'darwin' ? 'libuniffi_fixture_simple.dylib' : 'libuniffi_fixture_simple.so'
);

const CRATE = 'uniffi_fixture_simple';

const SYMBOLS = {
  rustbufferAlloc: `ffi_${CRATE}_rustbuffer_alloc`,
  rustbufferFree: `ffi_${CRATE}_rustbuffer_free`,
  rustbufferFromBytes: `ffi_${CRATE}_rustbuffer_from_bytes`,
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
