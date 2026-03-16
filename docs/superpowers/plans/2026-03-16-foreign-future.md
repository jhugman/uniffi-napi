# Foreign Future Protocol Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable uniffi-napi to handle UniFFI's async foreign trait protocol — C function pointers received as callback arguments, struct-by-value passing via libffi, and mutable struct out-pointer pass-through.

**Architecture:** Three changes to the existing callback/struct infrastructure: (1) `ffi_type_for` gains struct definition context to build `Type::structure()` for by-value structs, (2) VTable trampolines wrap incoming C function pointers as callable JS functions that marshal arguments (including by-value structs) and invoke via `libffi::middle::Cif::call()`, (3) `MutReference(Struct)` arguments pass through as opaque bigints. Each change is tested incrementally, culminating in the existing AsyncFetcher fixture test.

**Tech Stack:** Rust (napi-rs, libffi), Node.js test runner, UniFFI 0.31

**Spec:** `docs/superpowers/specs/2026-03-16-foreign-future-design.md`

---

## File Structure

### Modified files

| File | Responsibility | Change |
|------|---------------|--------|
| `src/cif.rs` | FfiType → libffi Type mapping | `ffi_type_for` accepts struct defs, `Struct(name)` builds `Type::structure()` |
| `src/callback.rs` | Callback trampolines + arg marshalling | `RawCallbackArg::Pointer` variant; `c_arg_to_js` and `read_raw_arg` handle `Callback` args (fn pointers) and `MutReference` args |
| `src/structs.rs` | VTable construction + trampoline dispatch | VTable trampolines handle `Callback`-typed args as fn pointer wrappers |
| `src/register.rs` | Registration pipeline | Passes struct defs to CIF construction; pre-computes libffi struct types |
| `tests/fixture.test.mjs` | Integration tests | Unskip AsyncFetcher test, add struct-by-value and fn pointer tests |

### New files

| File | Responsibility |
|------|---------------|
| `src/fn_pointer.rs` | C function pointer → JS callable wrapper: CIF construction, JS→C argument marshalling (including struct-by-value), invocation via `Cif::call()` |

---

## Chunk 1: Struct-by-Value Type Support

### Task 1: Make `ffi_type_for` support `Struct(name)` via struct definitions

Currently `ffi_type_for(&FfiTypeDesc) -> Type` panics on `Struct(name)`. We need it to look up the struct definition and build `Type::structure()`. This changes its signature to accept a struct definition map.

**Files:**
- Modify: `src/cif.rs`
- Modify: `src/callback.rs` (call site)
- Modify: `src/register.rs` (call site)
- Modify: `src/structs.rs` (call site)

- [ ] **Step 1: Change `ffi_type_for` signature to accept struct definitions**

Change the function signature from:
```rust
pub fn ffi_type_for(desc: &FfiTypeDesc) -> Type
```
to:
```rust
pub fn ffi_type_for(desc: &FfiTypeDesc, struct_defs: &HashMap<String, StructDef>) -> Type
```

Where `StructDef` is `structs::StructDef` (already exists — it's the parsed struct field definitions).

For the `Struct(name)` arm, look up the struct definition and build a `Type::structure()`:
```rust
FfiTypeDesc::Struct(name) => {
    let struct_def = struct_defs.get(name).unwrap_or_else(|| {
        panic!("Unknown struct type: '{name}'. Ensure it is defined in the structs section of register().")
    });
    Type::structure(
        struct_def.fields.iter().map(|f| ffi_type_for(&f.field_type, struct_defs)).collect()
    )
}
```

Note: This is recursive — a struct field can itself be `Struct(name)` (e.g., `RustCallStatus` inside `ForeignFutureResult`).

- [ ] **Step 2: Update all call sites**

The function is called in four files. Update each to pass the struct definitions:

1. `src/cif.rs` — only the function definition changes (no internal calls)
2. `src/register.rs` — `call_ffi_function` calls `ffi_type_for` at line 151 and 155. The `struct_defs` are already available as a parameter.
3. `src/callback.rs` — `build_callback_cif` at line 592. This function needs to accept struct defs too:
   ```rust
   pub fn build_callback_cif(callback_def: &CallbackDef, struct_defs: &HashMap<String, StructDef>) -> Cif {
       let cif_arg_types: Vec<Type> = callback_def.args.iter().map(|a| ffi_type_for(a, struct_defs)).collect();
       let cif_ret_type = ffi_type_for(&callback_def.ret, struct_defs);
       Cif::new(cif_arg_types, cif_ret_type)
   }
   ```
   Update call sites in `register.rs` (line 358).
4. `src/structs.rs` — `build_vtable_struct` calls `ffi_type_for` in CIF construction for each callback field. Pass struct defs through.

For now, pass an **empty HashMap** at all call sites where struct defs aren't available. No by-value structs are used in existing code paths, so the `Struct(name)` arm won't be hit. This keeps existing tests passing.

- [ ] **Step 3: Verify existing tests still pass**

```bash
npm run build:debug && npm test
```

Expected: All 42 tests pass, 1 skipped. The signature change is internal — no JS API changes.

- [ ] **Step 4: Commit**

```bash
git add src/cif.rs src/callback.rs src/register.rs src/structs.rs
git commit -m "refactor: ffi_type_for accepts struct definitions for by-value struct support"
```

---

### Task 2: Wire real struct_defs through all ffi_type_for call sites

The `StructDef` and `StructField` types already exist in `src/structs.rs` with the right shape — `StructField` has `name: String` and `field_type: FfiTypeDesc`, and `parse_structs` already parses all field types (not just callbacks). No type changes needed.

This task replaces the empty HashMaps from Task 1 with the real `struct_defs`.

**Files:**
- Modify: `src/register.rs` (pass struct defs to CIF construction)

- [ ] **Step 1: Pass real struct_defs to ffi_type_for everywhere**

In `register.rs`, replace the empty HashMap from Task 1 with the actual `struct_defs` parsed from `parse_structs`. Update `call_ffi_function` and `build_callback_cif` call sites.

- [ ] **Step 4: Verify existing tests still pass**

```bash
npm run build:debug && npm test
```

Expected: All 42 pass, 1 skipped. No behavioral change yet — just plumbing.

- [ ] **Step 5: Commit**

```bash
git add src/structs.rs src/register.rs
git commit -m "feat: parse data struct definitions and wire struct_defs through CIF construction"
```

---

### Task 3: Write a unit test for struct-by-value CIF construction

Verify that `ffi_type_for(Struct("name"), struct_defs)` produces the correct `Type::structure()`.

**Files:**
- Modify: `tests/fixture.test.mjs`

- [ ] **Step 1: Add a minimal test that registers a data struct**

Add a test that registers a function taking a `FfiType.Struct('SimpleStruct')` argument, with the struct defined in the `structs` section. This will exercise the `ffi_type_for` → `Type::structure()` path during CIF construction at registration time.

We can't yet _call_ this function (we don't have marshalling), but we can verify that `register()` itself doesn't crash.

```js
test('fixture: register with struct-by-value does not crash', () => {
  const mod = UniffiNativeModule.open(LIB_PATH);
  // Register a function with a struct-by-value argument.
  // We use a dummy function name that won't be called — just testing registration.
  // Actually, we need a real symbol. Use the rustbuffer_alloc symbol as a dummy.
  // Or better: just verify the struct definition is parsed without error
  // by registering a callback that takes a Struct arg.
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
```

- [ ] **Step 2: Run and verify**

```bash
npm run build:debug && npm test -- --test-name-pattern "struct-by-value"
```

Expected: PASS. The struct definition is parsed and `Type::structure()` is built without panicking.

- [ ] **Step 3: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: verify struct-by-value CIF construction during registration"
```

---

## Chunk 2: C Function Pointer Wrapping

### Task 4: Create `src/fn_pointer.rs` — wrapping C function pointers as callable JS functions

When a VTable callback receives a `Callback`-typed argument, the trampoline must wrap the C function pointer as a callable JS function. This module handles that wrapping.

**Files:**
- Create: `src/fn_pointer.rs`
- Modify: `src/lib.rs` (add module)

- [ ] **Step 1: Design the fn_pointer module**

The module needs:
1. A function that takes a raw C function pointer (`*const c_void`), a `CallbackDef` (describing its signature), struct definitions, and RustBuffer helpers, and returns a `napi::JsFunction`.
2. When the returned JS function is called, it:
   a. Marshals each JS argument to its C representation (including struct-by-value)
   b. Builds a libffi `Arg` array
   c. Calls the C function pointer via `Cif::call()`

- [ ] **Step 2: Implement JS→C struct marshalling**

This is the core of struct-by-value support. Given a JS object and a `StructDef`, produce a `Vec<u8>` buffer with the correct C struct layout.

```rust
/// Marshal a JS object into a C struct byte buffer matching the libffi struct layout.
///
/// Uses libffi's Type::structure() to determine field sizes, alignments, and offsets.
/// Fields are read from the JS object by name and written at the correct byte offset.
pub fn marshal_js_struct_to_bytes(
    env: &Env,
    js_obj: &JsObject,
    struct_def: &StructDef,
    struct_defs: &HashMap<String, StructDef>,
    rb_from_bytes_ptr: *const c_void,
) -> Result<Vec<u8>> {
    // 1. Build the libffi Type::structure() to get size and field offsets
    // 2. Allocate a Vec<u8> of the struct's total size (from Type::as_raw_ptr() -> ffi_type.size)
    // 3. For each field:
    //    a. Read the JS property by name
    //    b. Convert to C bytes based on field type
    //    c. Write at the correct offset
    // 4. Return the buffer
}
```

Key detail: Use `libffi::raw::ffi_type` to query struct size and field offsets. The `Type::structure()` call populates the internal `ffi_type` with `size`, `alignment`, and element offsets. Access these via the raw pointer from `Type::as_raw_ptr()`.

The field offset computation: libffi doesn't expose individual offsets directly, but you can compute them by iterating the struct's `elements` array and accumulating sizes with alignment padding. Alternatively, use `libffi::raw::ffi_get_struct_offsets()` if available, or compute manually.

- [ ] **Step 3: Implement `create_fn_pointer_wrapper`**

```rust
/// Wrap a C function pointer as a callable JS function.
///
/// The returned JS function, when called from JS, marshals its arguments
/// to C representations and invokes the C function pointer via libffi.
pub fn create_fn_pointer_wrapper(
    env: &Env,
    fn_ptr: *const c_void,
    cb_def: &CallbackDef,
    struct_defs: &HashMap<String, StructDef>,
    rb_from_bytes_ptr: *const c_void,
    rb_free_ptr: *const c_void,
) -> Result<JsFunction> {
    // 1. Build a CIF from the callback definition
    // 2. Create a JS function (env.create_function_from_closure) that:
    //    a. Reads each JS argument
    //    b. For Struct args: call marshal_js_struct_to_bytes
    //    c. For RustBuffer args: call js_uint8array_to_rust_buffer
    //    d. For scalars: use marshal::js_to_boxed
    //    e. Build Arg array
    //    f. Call cif.call(CodePtr::from_ptr(fn_ptr), &args)
    // 3. Return the JS function
}
```

The CIF and other static data should be pre-computed and captured by the closure via `Rc`, matching the pattern used in `register.rs:158-179`.

- [ ] **Step 4: Add module to lib.rs**

```rust
mod fn_pointer;
```

- [ ] **Step 5: Verify it compiles**

```bash
npm run build:debug
```

Expected: Compiles. No tests yet — the module isn't wired into the trampoline.

- [ ] **Step 6: Commit**

```bash
git add src/fn_pointer.rs src/lib.rs
git commit -m "feat: add fn_pointer module — wrap C function pointers as callable JS functions"
```

---

### Task 5: Wire fn pointer wrapping into the VTable trampoline

When the VTable trampoline encounters a `Callback`-typed argument (not a struct field, but a callback argument), it should wrap the C function pointer as a callable JS function using `fn_pointer::create_fn_pointer_wrapper`.

**Files:**
- Modify: `src/structs.rs` (VTable trampoline same-thread and cross-thread paths)
- Modify: `src/callback.rs` (simple callback trampoline — `c_arg_to_js` and `read_raw_arg`)

- [ ] **Step 1: Add `RawCallbackArg::Pointer` variant**

In `src/callback.rs`, add a new variant for transporting raw pointers across threads:
```rust
pub enum RawCallbackArg {
    // ... existing variants ...
    /// A raw C pointer (function pointer or opaque reference) transported as usize.
    /// Used for C function pointers received as callback arguments.
    Pointer(usize),
}
```

- [ ] **Step 2: Handle `Callback` in `read_raw_arg`**

In `src/callback.rs`, add a case for `FfiTypeDesc::Callback(_)` in `read_raw_arg`:
```rust
FfiTypeDesc::Callback(_) => {
    // Read the raw function pointer value
    let fn_ptr = *(arg_ptr as *const *const c_void);
    Some(RawCallbackArg::Pointer(fn_ptr as usize))
}
```

- [ ] **Step 3: Handle `Callback` in `c_arg_to_js`**

In `src/callback.rs`, `c_arg_to_js` just transports the pointer as a bigint. The actual wrapping as a callable JS function happens in the VTable trampoline (`structs.rs`), which has access to `callback_defs` and `struct_defs` via `VTableTrampolineUserdata`.

```rust
FfiTypeDesc::Callback(_) => {
    let fn_ptr = *(arg_ptr as *const *const c_void);
    Ok(env.create_bigint_from_u64(fn_ptr as u64)?.into_unknown()?)
}
```

- [ ] **Step 4: Handle `MutReference` in `read_raw_arg` and `c_arg_to_js`**

Add cases for `FfiTypeDesc::MutReference(_)`:
```rust
// In read_raw_arg:
FfiTypeDesc::MutReference(_) | FfiTypeDesc::Reference(_) => {
    let ptr = *(arg_ptr as *const *const c_void);
    Some(RawCallbackArg::Pointer(ptr as usize))
}

// In c_arg_to_js:
FfiTypeDesc::MutReference(_) | FfiTypeDesc::Reference(_) => {
    let ptr = *(arg_ptr as *const *const c_void);
    Ok(env.create_bigint_from_u64(ptr as u64)?.into_unknown()?)
}
```

- [ ] **Step 5: Handle `Pointer` in `raw_arg_to_js`**

```rust
RawCallbackArg::Pointer(v) => Ok(env.create_bigint_from_u64(*v as u64)?.into_unknown()?),
```

- [ ] **Step 6: Wire fn pointer wrapping into VTable trampoline (same-thread path)**

In `src/structs.rs`, the `vtable_trampoline_main_thread` function converts C args to JS args. After converting a `Callback`-typed arg to a raw pointer (bigint), post-process it by wrapping it:

`VTableTrampolineUserdata` already has `rb_from_bytes_ptr` and `rb_free_ptr`. Add two new fields for callback and struct definitions:

```rust
// Add to VTableTrampolineUserdata:
pub callback_defs: HashMap<String, CallbackDef>,
pub struct_defs: HashMap<String, StructDef>,
```

Then in the same-thread path, after reading the arg:
```rust
FfiTypeDesc::Callback(cb_name) => {
    let fn_ptr = *(arg_ptr as *const *const c_void);
    let cb_def = userdata.callback_defs.get(cb_name).expect("Unknown callback");
    let js_func = fn_pointer::create_fn_pointer_wrapper(
        &env, fn_ptr, cb_def,
        &userdata.struct_defs,
        userdata.rb_from_bytes_ptr,
        userdata.rb_free_ptr,
    )?;
    js_func.into_unknown()
}
```

For the cross-thread path, the pointer is transported via `RawCallbackArg::Pointer` and the wrapping happens on the main thread in the TSF callback.

- [ ] **Step 7: Update `build_vtable_struct` to pass new data into userdata**

In `src/structs.rs`, `build_vtable_struct` already receives `callback_defs` and `rb_from_bytes_ptr`/`rb_free_ptr`. Add `struct_defs` as an additional parameter.

When constructing each `VTableTrampolineUserdata`, clone `callback_defs` and `struct_defs` into the userdata struct.

Update the call site in `register.rs` to pass `struct_defs`.

- [ ] **Step 8: Verify it compiles and existing tests pass**

```bash
npm run build:debug && npm test
```

Expected: All 42 pass, 1 skipped. The new code paths aren't exercised yet by existing tests.

- [ ] **Step 9: Commit**

```bash
git add src/callback.rs src/structs.rs src/fn_pointer.rs src/register.rs
git commit -m "feat: VTable trampoline wraps C function pointers as callable JS functions"
```

---

## Chunk 3: End-to-End AsyncFetcher Test

### Task 6: Write the AsyncFetcher integration test

This test exercises the full foreign future protocol: VTable registration with async method, receiving a C function pointer, calling it with a struct-by-value argument.

**Files:**
- Modify: `tests/fixture.test.mjs`

- [ ] **Step 1: Replace the skipped AsyncFetcher test with the real implementation**

Remove the existing skipped test (with the TODO comments) and replace it with:

```js
test('fixture: AsyncFetcher.fetch (async foreign trait)', async () => {
  const nm = openAndRegister(
    {
      [`uniffi_${CRATE}_fn_init_callback_vtable_asyncfetcher`]: {
        args: [FfiType.Reference(FfiType.Struct('VTable_AsyncFetcher'))],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      [`uniffi_${CRATE}_fn_func_use_async_fetcher`]: {
        args: [FfiType.Handle, FfiType.RustBuffer],
        ret: FfiType.Handle,
        hasRustCallStatus: true,
      },
      [`ffi_${CRATE}_rust_future_poll_rust_buffer`]: {
        args: [FfiType.Handle, FfiType.Callback('rust_future_continuation'), FfiType.UInt64],
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
      callback_asyncfetcher_free: {
        args: [FfiType.UInt64],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      callback_asyncfetcher_clone: {
        args: [FfiType.UInt64],
        ret: FfiType.UInt64,
        hasRustCallStatus: false,
      },
      callback_asyncfetcher_fetch: {
        args: [
          FfiType.UInt64,
          FfiType.RustBuffer,
          FfiType.Callback('ForeignFutureCompleteRustBuffer'),
          FfiType.UInt64,
          FfiType.MutReference(FfiType.Struct('ForeignFutureDroppedCallbackStruct')),
        ],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      ForeignFutureCompleteRustBuffer: {
        args: [FfiType.UInt64, FfiType.Struct('ForeignFutureResultRustBuffer')],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
      ForeignFutureDroppedCallback: {
        args: [FfiType.UInt64],
        ret: FfiType.Void,
        hasRustCallStatus: false,
      },
    },
    {
      VTable_AsyncFetcher: [
        { name: 'uniffi_free', type: FfiType.Callback('callback_asyncfetcher_free') },
        { name: 'uniffi_clone', type: FfiType.Callback('callback_asyncfetcher_clone') },
        { name: 'fetch', type: FfiType.Callback('callback_asyncfetcher_fetch') },
      ],
      ForeignFutureResultRustBuffer: [
        { name: 'returnValue', type: FfiType.RustBuffer },
        { name: 'callStatus', type: FfiType.Struct('RustCallStatus') },
      ],
      RustCallStatus: [
        { name: 'code', type: FfiType.Int8 },
        { name: 'errorBuf', type: FfiType.RustBuffer },
      ],
      ForeignFutureDroppedCallbackStruct: [
        { name: 'callbackData', type: FfiType.UInt64 },
        { name: 'callback', type: FfiType.Callback('ForeignFutureDroppedCallback') },
      ],
    },
  );

  // Register the AsyncFetcher VTable
  nm[`uniffi_${CRATE}_fn_init_callback_vtable_asyncfetcher`]({
    uniffi_free: (handle) => {},
    uniffi_clone: (handle) => handle,
    fetch: (handle, inputBuf, completeCb, completeCbData, outDroppedCb) => {
      // Implement the async fetch: prepend "fetched: " to the input string
      const input = liftString(inputBuf);
      const result = `fetched: ${input}`;

      // Call the completion callback with the result
      // completeCb is a callable JS function wrapping the C function pointer.
      // It takes (u64 callbackData, ForeignFutureResultRustBuffer { returnValue, callStatus })
      completeCb(
        completeCbData,
        {
          returnValue: lowerString(result),
          callStatus: { code: 0, errorBuf: new Uint8Array(0) },
        }
      );
    },
  });

  // Call use_async_fetcher — this is an async function, so poll it
  const result = await uniffiRustCallAsync(nm, {
    rustFutureFunc: () => {
      const status = { code: 0 };
      return nm[`uniffi_${CRATE}_fn_func_use_async_fetcher`](1n, lowerString('hello'), status);
    },
    pollFunc: `ffi_${CRATE}_rust_future_poll_rust_buffer`,
    completeFunc: `ffi_${CRATE}_rust_future_complete_rust_buffer`,
    freeFunc: `ffi_${CRATE}_rust_future_free_rust_buffer`,
    liftFunc: liftString,
  });

  assert.strictEqual(result, 'fetched: hello');
});
```

**Important:** The exact symbol names and VTable field order must match the actual `nm` output from the fixture. The names above are predictions based on Task 4 of the previous plan. If they differ, adjust accordingly.

- [ ] **Step 2: Run the test**

```bash
npm run build:debug && npm test -- --test-name-pattern "AsyncFetcher"
```

Expected: PASS. This exercises the full protocol:
1. VTable registration with async `fetch` method
2. Rust calls `fetch` → JS receives `completeCb` as a callable function
3. JS calls `completeCb(data, { returnValue, callStatus })` → struct marshalled by value
4. Rust receives the result via the oneshot channel
5. The async future completes and the polling loop returns the result

- [ ] **Step 3: Run full test suite**

```bash
npm test
```

Expected: All 43 tests pass (42 existing + 1 new AsyncFetcher, previously skipped now passing).

- [ ] **Step 4: Commit**

```bash
git add tests/fixture.test.mjs
git commit -m "test: AsyncFetcher async foreign trait — end-to-end via foreign future protocol"
```

---

## Notes for the Implementer

### libffi struct layout queries

To get struct field offsets from a `Type::structure()`, you need to access the raw `ffi_type` pointer. The `libffi` crate's `Type` has an `as_raw_ptr()` method that returns `*mut ffi_type`. From there:
- `(*raw_ptr).size` gives the total struct size
- `(*raw_ptr).alignment` gives the struct alignment
- Field offsets can be computed by iterating the `elements` array (null-terminated array of `*mut ffi_type` pointers) and accumulating sizes with padding

Alternatively, use a helper function:
```rust
fn struct_field_offsets(struct_type: &Type) -> Vec<usize> {
    let raw = struct_type.as_raw_ptr();
    let mut offsets = Vec::new();
    let mut offset = 0usize;
    unsafe {
        let elements = (*raw).elements;
        let mut i = 0;
        while !(*elements.add(i)).is_null() {
            let field_type = *elements.add(i);
            let field_align = (*field_type).alignment as usize;
            // Align offset
            offset = (offset + field_align - 1) & !(field_align - 1);
            offsets.push(offset);
            offset += (*field_type).size;
            i += 1;
        }
    }
    offsets
}
```

### RustBuffer inside struct-by-value

When `ForeignFutureResult` contains a `RustBuffer` field (`returnValue`), the JS side provides a `Uint8Array`. The marshalling must convert this to a `RustBufferC` struct (24 bytes: `{u64 capacity, u64 len, *mut u8 data}`) and write it at the correct field offset.

Use `rustbuffer_from_bytes` (the same function used in `js_uint8array_to_rust_buffer` in `register.rs`) to create the `RustBufferC`, then copy its bytes into the struct buffer.

### The `RustCallStatus` as a nested struct

`RustCallStatus` inside `ForeignFutureResult` is `{i8 code, RustBuffer error_buf}`. This is a 32-byte nested struct (1 byte + padding + 24 bytes RustBuffer). The JS side passes `{ code: 0, errorBuf: new Uint8Array(0) }`.

When marshalling, recursively handle nested `Struct` fields: look up the nested struct definition, marshal its JS sub-object, and write the result at the parent field's offset.

### Debugging tips

If the AsyncFetcher test hangs:
1. Add `console.log` in the `fetch` callback to verify it's called
2. Check that `completeCb` is actually a function (not a bigint or undefined)
3. Verify the struct field order matches UniFFI's layout by checking the Rust source
4. Check that the `RustCallStatus` within the result has `code: 0`

If you get a segfault when calling `completeCb`:
1. The struct byte layout may be wrong — check field offsets and sizes
2. The `RustBuffer` within the struct may not be correctly marshalled
3. The function pointer may not be correctly extracted from the C arg
