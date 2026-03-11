import { test } from 'node:test';
import assert from 'node:assert';
import { join } from 'node:path';
import lib from '../lib.js';
const { UniffiNativeModule } = lib;

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
