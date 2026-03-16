use std::collections::HashMap;
use std::ffi::c_void;

use libffi::low;
use libffi::middle::{Cif, Closure, Type};
use napi::{Env, JsObject, JsUnknown, NapiRaw, NapiValue, Result};

use crate::callback::{
    c_arg_to_js, js_return_to_raw, raw_arg_to_js, read_raw_arg, CallbackDef, RawCallbackArg,
};
use crate::cif::ffi_type_for;
use crate::ffi_c_types::{ForeignBytesC, RustBufferC, RustBufferFromBytesFn, RustCallStatusC};
use crate::ffi_type::FfiTypeDesc;
use crate::is_main_thread;
use crate::napi_utils;
use napi::threadsafe_function::{ErrorStrategy, ThreadsafeFunction, ThreadsafeFunctionCallMode};

/// Packed arguments for cross-thread VTable callback dispatch.
struct VTableCallRequest {
    /// C argument values, read from raw pointers on the calling thread.
    args: Vec<RawCallbackArg>,
    /// If has_rust_call_status, the initial code value from C.
    rust_call_status_code: i8,
    /// Channel to send the result back to the calling thread.
    /// None for fire-and-forget (non-blocking) callbacks.
    response_tx: Option<std::sync::mpsc::SyncSender<VTableCallResponse>>,
}

/// Result sent back from the JS thread to the calling thread.
struct VTableCallResponse {
    /// Return value (using RawCallbackArg for type-safe transport).
    return_value: Option<RawCallbackArg>,
    /// Updated RustCallStatus code from JS.
    rust_call_status_code: i8,
}

/// A single field in a struct definition.
#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub field_type: FfiTypeDesc,
}

/// A parsed struct definition (list of fields).
#[derive(Debug, Clone)]
pub struct StructDef {
    pub fields: Vec<StructField>,
}

/// Parse the `structs` map from JS definitions.
/// Each struct is an array of { name, type } objects.
pub fn parse_structs(env: &Env, definitions: &JsObject) -> Result<HashMap<String, StructDef>> {
    let mut map = HashMap::new();

    let has_structs: bool = definitions.has_named_property("structs")?;
    if !has_structs {
        return Ok(map);
    }

    let structs: JsObject = definitions.get_named_property("structs")?;
    let names = structs.get_property_names()?;
    let len = names.get_array_length()?;

    for i in 0..len {
        let name: String = names
            .get_element::<napi::JsString>(i)?
            .into_utf8()?
            .as_str()?
            .to_owned();
        let fields_arr: JsObject = structs.get_named_property(&name)?;
        let fields_len = fields_arr.get_array_length()?;
        let mut fields = Vec::with_capacity(fields_len as usize);

        for j in 0..fields_len {
            let field_obj: JsObject = fields_arr.get_element(j)?;
            let field_name: String = field_obj.get_named_property("name")?;
            let type_obj: JsObject = field_obj.get_named_property("type")?;
            let field_type = FfiTypeDesc::from_js_object(env, &type_obj)?;
            fields.push(StructField {
                name: field_name,
                field_type,
            });
        }

        map.insert(name, StructDef { fields });
    }

    Ok(map)
}

/// Userdata for VTable callback trampolines.
/// Unlike TrampolineUserdata, this supports return values and RustCallStatus handling.
/// Stores a persistent reference to the JS function so it survives GC.
pub struct VTableTrampolineUserdata {
    pub raw_env: napi::sys::napi_env,
    pub fn_ref: napi::sys::napi_ref,
    /// The declared arg types from the callback definition (not including RustCallStatus).
    pub arg_types: Vec<FfiTypeDesc>,
    /// The return type of this callback.
    pub ret_type: FfiTypeDesc,
    /// Whether the last C arg is a &mut RustCallStatus.
    pub has_rust_call_status: bool,
    tsfn: Option<ThreadsafeFunction<VTableCallRequest, ErrorStrategy::Fatal>>,
    pub rb_from_bytes_ptr: *const c_void,
    pub rb_free_ptr: *const c_void,
}

// Safety: raw_env and raw_fn are only accessed on the main thread.
unsafe impl Send for VTableTrampolineUserdata {}
unsafe impl Sync for VTableTrampolineUserdata {}

/// The trampoline for VTable callbacks. Handles return values and RustCallStatus.
pub unsafe extern "C" fn vtable_trampoline_callback(
    _cif: &low::ffi_cif,
    result: &mut c_void,
    args: *const *const c_void,
    userdata: &VTableTrampolineUserdata,
) {
    if !is_main_thread() {
        // Cross-thread path: serialize args, dispatch to JS thread, wait for result
        vtable_trampoline_cross_thread(result, args, userdata);
        return;
    }

    // Main-thread path: call JS directly (existing behavior)
    vtable_trampoline_main_thread(result, args, userdata);
}

unsafe fn vtable_trampoline_main_thread(
    result: &mut c_void,
    args: *const *const c_void,
    userdata: &VTableTrampolineUserdata,
) {
    let env = Env::from_raw(userdata.raw_env);

    let mut raw_fn: napi::sys::napi_value = std::ptr::null_mut();
    let status =
        napi::sys::napi_get_reference_value(userdata.raw_env, userdata.fn_ref, &mut raw_fn);
    if status != napi::sys::Status::napi_ok || raw_fn.is_null() {
        return;
    }

    let js_fn = match napi::JsFunction::from_raw(userdata.raw_env, raw_fn) {
        Ok(f) => f,
        Err(_) => return,
    };

    let declared_count = userdata.arg_types.len();
    let mut js_args: Vec<napi::JsUnknown> = Vec::with_capacity(declared_count + 1);

    for (i, desc) in userdata.arg_types.iter().enumerate() {
        let arg_ptr = *args.add(i);
        let js_val = match c_arg_to_js(&env, desc, arg_ptr, userdata.rb_free_ptr) {
            Ok(v) => v,
            Err(_) => return,
        };
        js_args.push(js_val);
    }

    let mut status_ptr: *mut RustCallStatusForVTable = std::ptr::null_mut();
    if userdata.has_rust_call_status {
        let rcs_arg_ptr = *args.add(declared_count);
        status_ptr = *(rcs_arg_ptr as *const *mut RustCallStatusForVTable);

        let code = if !status_ptr.is_null() {
            (*status_ptr).code as i32
        } else {
            0
        };

        let mut js_status = match env.create_object() {
            Ok(o) => o,
            Err(_) => return,
        };
        if js_status
            .set_named_property("code", env.create_int32(code).unwrap())
            .is_err()
        {
            return;
        }
        js_args.push(js_status.into_unknown());
    }

    let call_result = js_fn.call(None, &js_args);

    if userdata.has_rust_call_status && !status_ptr.is_null() {
        if let Some(js_status_unknown) = js_args.last() {
            if let Ok(js_status_obj) = JsObject::from_raw(userdata.raw_env, js_status_unknown.raw())
            {
                if let Ok(code_val) = js_status_obj.get_named_property::<i32>("code") {
                    (*status_ptr).code = code_val as i8;
                }
            }
        }
    }

    if let Ok(js_ret) = call_result {
        write_return_value(
            result,
            &userdata.ret_type,
            userdata.raw_env,
            js_ret,
            userdata.rb_from_bytes_ptr,
        );
    }
}

unsafe fn vtable_trampoline_cross_thread(
    result: &mut c_void,
    args: *const *const c_void,
    userdata: &VTableTrampolineUserdata,
) {
    let tsfn = match &userdata.tsfn {
        Some(t) => t,
        None => return,
    };

    // Read C args into portable values
    let declared_count = userdata.arg_types.len();
    let mut raw_args = Vec::with_capacity(declared_count);
    for (i, desc) in userdata.arg_types.iter().enumerate() {
        let arg_ptr = *args.add(i);
        let raw_arg = match read_raw_arg(desc, arg_ptr, userdata.rb_free_ptr) {
            Some(a) => a,
            None => return,
        };
        raw_args.push(raw_arg);
    }

    // Implicit rule: needs_blocking if callback returns a value or has RustCallStatus
    let needs_blocking =
        !matches!(userdata.ret_type, FfiTypeDesc::Void) || userdata.has_rust_call_status;

    // Read RustCallStatus code if present
    let mut status_ptr: *mut RustCallStatusForVTable = std::ptr::null_mut();
    let rcs_code = if userdata.has_rust_call_status {
        let rcs_arg_ptr = *args.add(declared_count);
        status_ptr = *(rcs_arg_ptr as *const *mut RustCallStatusForVTable);
        if !status_ptr.is_null() {
            (*status_ptr).code
        } else {
            0
        }
    } else {
        0
    };

    if needs_blocking {
        // Blocking path: create rendezvous channel, wait for JS thread response
        let (tx, rx) = std::sync::mpsc::sync_channel(1);

        let request = VTableCallRequest {
            args: raw_args,
            rust_call_status_code: rcs_code,
            response_tx: Some(tx),
        };

        tsfn.call(request, ThreadsafeFunctionCallMode::Blocking);

        if let Ok(response) = rx.recv() {
            if let Some(ref raw_ret) = response.return_value {
                write_raw_return_value(
                    result,
                    &userdata.ret_type,
                    raw_ret,
                    userdata.rb_from_bytes_ptr,
                );
            }
            if userdata.has_rust_call_status && !status_ptr.is_null() {
                (*status_ptr).code = response.rust_call_status_code;
            }
        }
    } else {
        // Non-blocking path: fire-and-forget (e.g. free callbacks)
        let request = VTableCallRequest {
            args: raw_args,
            rust_call_status_code: rcs_code,
            response_tx: None,
        };

        tsfn.call(request, ThreadsafeFunctionCallMode::NonBlocking);
    }
}

/// Write the JS return value into the libffi result buffer.
/// For closures, libffi expects integer types smaller than `ffi_arg` to be
/// written as `ffi_arg` (typically u64 on 64-bit platforms).
unsafe fn write_return_value(
    result: &mut c_void,
    ret_type: &FfiTypeDesc,
    raw_env: napi::sys::napi_env,
    js_ret: napi::JsUnknown,
    rb_from_bytes_ptr: *const c_void,
) {
    let result_ptr = result as *mut c_void;
    match ret_type {
        FfiTypeDesc::Void => {}
        FfiTypeDesc::Int8 => {
            if let Ok(num) = napi::JsNumber::from_raw(raw_env, js_ret.raw()) {
                if let Ok(v) = num.get_double() {
                    *(result_ptr as *mut low::ffi_sarg) = v as i8 as low::ffi_sarg;
                }
            }
        }
        FfiTypeDesc::UInt8 => {
            if let Ok(num) = napi::JsNumber::from_raw(raw_env, js_ret.raw()) {
                if let Ok(v) = num.get_double() {
                    *(result_ptr as *mut low::ffi_arg) = v as u8 as low::ffi_arg;
                }
            }
        }
        FfiTypeDesc::Int16 => {
            if let Ok(num) = napi::JsNumber::from_raw(raw_env, js_ret.raw()) {
                if let Ok(v) = num.get_double() {
                    *(result_ptr as *mut low::ffi_sarg) = v as i16 as low::ffi_sarg;
                }
            }
        }
        FfiTypeDesc::UInt16 => {
            if let Ok(num) = napi::JsNumber::from_raw(raw_env, js_ret.raw()) {
                if let Ok(v) = num.get_double() {
                    *(result_ptr as *mut low::ffi_arg) = v as u16 as low::ffi_arg;
                }
            }
        }
        FfiTypeDesc::Int32 => {
            if let Ok(num) = napi::JsNumber::from_raw(raw_env, js_ret.raw()) {
                if let Ok(v) = num.get_double() {
                    *(result_ptr as *mut low::ffi_sarg) = v as i32 as low::ffi_sarg;
                }
            }
        }
        FfiTypeDesc::UInt32 => {
            if let Ok(num) = napi::JsNumber::from_raw(raw_env, js_ret.raw()) {
                if let Ok(v) = num.get_double() {
                    *(result_ptr as *mut low::ffi_arg) = v as u32 as low::ffi_arg;
                }
            }
        }
        FfiTypeDesc::Int64 => {
            if let Ok(bigint) = napi::JsBigInt::from_raw(raw_env, js_ret.raw()) {
                if let Ok((v, _)) = bigint.get_i64() {
                    *(result_ptr as *mut i64) = v;
                }
            }
        }
        FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => {
            if let Ok(bigint) = napi::JsBigInt::from_raw(raw_env, js_ret.raw()) {
                if let Ok((v, _)) = bigint.get_u64() {
                    *(result_ptr as *mut u64) = v;
                }
            }
        }
        FfiTypeDesc::Float32 => {
            if let Ok(num) = napi::JsNumber::from_raw(raw_env, js_ret.raw()) {
                if let Ok(v) = num.get_double() {
                    *(result_ptr as *mut f32) = v as f32;
                }
            }
        }
        FfiTypeDesc::Float64 => {
            if let Ok(num) = napi::JsNumber::from_raw(raw_env, js_ret.raw()) {
                if let Ok(v) = num.get_double() {
                    *(result_ptr as *mut f64) = v;
                }
            }
        }
        FfiTypeDesc::RustBuffer => {
            // Extract Uint8Array data
            let (data, length) = match napi_utils::read_typedarray_data(raw_env, js_ret.raw()) {
                Some(v) => v,
                None => return, // Not a typed array — silently fail like other arms
            };

            // Create ForeignBytes and call rustbuffer_from_bytes
            if rb_from_bytes_ptr.is_null() || length > i32::MAX as usize {
                return;
            }
            let from_bytes: RustBufferFromBytesFn = std::mem::transmute(rb_from_bytes_ptr);
            let foreign = ForeignBytesC {
                len: length as i32,
                data: if length > 0 { data } else { std::ptr::null() },
            };
            let mut call_status = RustCallStatusC::default();
            let rb = from_bytes(foreign, &mut call_status as *mut _);
            if call_status.code != 0 {
                return;
            }

            // Write RustBufferC to result buffer
            *(result_ptr as *mut RustBufferC) = rb;
        }
        _ => {
            #[cfg(debug_assertions)]
            eprintln!("write_return_value: unsupported return type {:?}", ret_type);
        }
    }
}

/// Write a RawCallbackArg return value into the libffi result buffer.
/// Same ffi_arg/ffi_sarg widening rules as write_return_value.
unsafe fn write_raw_return_value(
    result: &mut c_void,
    ret_type: &FfiTypeDesc,
    raw_ret: &RawCallbackArg,
    rb_from_bytes_ptr: *const c_void,
) {
    let result_ptr = result as *mut c_void;
    match (ret_type, raw_ret) {
        (FfiTypeDesc::Int8, RawCallbackArg::Int8(v)) => {
            *(result_ptr as *mut low::ffi_sarg) = *v as low::ffi_sarg;
        }
        (FfiTypeDesc::UInt8, RawCallbackArg::UInt8(v)) => {
            *(result_ptr as *mut low::ffi_arg) = *v as low::ffi_arg;
        }
        (FfiTypeDesc::Int16, RawCallbackArg::Int16(v)) => {
            *(result_ptr as *mut low::ffi_sarg) = *v as low::ffi_sarg;
        }
        (FfiTypeDesc::UInt16, RawCallbackArg::UInt16(v)) => {
            *(result_ptr as *mut low::ffi_arg) = *v as low::ffi_arg;
        }
        (FfiTypeDesc::Int32, RawCallbackArg::Int32(v)) => {
            *(result_ptr as *mut low::ffi_sarg) = *v as low::ffi_sarg;
        }
        (FfiTypeDesc::UInt32, RawCallbackArg::UInt32(v)) => {
            *(result_ptr as *mut low::ffi_arg) = *v as low::ffi_arg;
        }
        (FfiTypeDesc::Int64, RawCallbackArg::Int64(v)) => {
            *(result_ptr as *mut i64) = *v;
        }
        (FfiTypeDesc::UInt64 | FfiTypeDesc::Handle, RawCallbackArg::UInt64(v)) => {
            *(result_ptr as *mut u64) = *v;
        }
        (FfiTypeDesc::Float32, RawCallbackArg::Float32(v)) => {
            *(result_ptr as *mut f32) = *v;
        }
        (FfiTypeDesc::Float64, RawCallbackArg::Float64(v)) => {
            *(result_ptr as *mut f64) = *v;
        }
        (FfiTypeDesc::RustBuffer, RawCallbackArg::RustBuffer(data)) => {
            if rb_from_bytes_ptr.is_null() {
                return;
            }

            // rustbuffer_from_bytes is a pure C function, safe to call from the calling thread.
            let from_bytes: RustBufferFromBytesFn = std::mem::transmute(rb_from_bytes_ptr);
            if data.len() > i32::MAX as usize {
                return;
            }
            let foreign = ForeignBytesC {
                len: data.len() as i32,
                data: if data.is_empty() {
                    std::ptr::null()
                } else {
                    data.as_ptr()
                },
            };
            let mut call_status = RustCallStatusC::default();
            let rb = from_bytes(foreign, &mut call_status as *mut _);
            if call_status.code == 0 {
                *(result_ptr as *mut RustBufferC) = rb;
            }
        }
        _ => {
            #[cfg(debug_assertions)]
            eprintln!(
                "write_raw_return_value: unsupported type pair {:?} / {:?}",
                ret_type, raw_ret
            );
        }
    }
}

/// Handler that runs on the JS thread when a VTableCallRequest is received via TSF.
fn vtable_tsfn_handler(env: &Env, userdata: &VTableTrampolineUserdata, request: VTableCallRequest) {
    let send_default = |req: &VTableCallRequest| {
        if let Some(ref tx) = req.response_tx {
            let _ = tx.send(VTableCallResponse {
                return_value: None,
                rust_call_status_code: req.rust_call_status_code,
            });
        }
    };

    let mut raw_fn: napi::sys::napi_value = std::ptr::null_mut();
    let status = unsafe {
        napi::sys::napi_get_reference_value(userdata.raw_env, userdata.fn_ref, &mut raw_fn)
    };
    if status != napi::sys::Status::napi_ok || raw_fn.is_null() {
        send_default(&request);
        return;
    }

    let js_fn = match unsafe { napi::JsFunction::from_raw(userdata.raw_env, raw_fn) } {
        Ok(f) => f,
        Err(_) => {
            send_default(&request);
            return;
        }
    };

    let mut js_args: Vec<napi::JsUnknown> = Vec::with_capacity(request.args.len() + 1);
    for raw_arg in &request.args {
        match raw_arg_to_js(env, raw_arg) {
            Ok(v) => js_args.push(v),
            Err(_) => {
                send_default(&request);
                return;
            }
        }
    }

    if userdata.has_rust_call_status {
        let mut js_status = match env.create_object() {
            Ok(o) => o,
            Err(_) => {
                send_default(&request);
                return;
            }
        };
        let code_val = match env.create_int32(request.rust_call_status_code as i32) {
            Ok(v) => v,
            Err(_) => {
                send_default(&request);
                return;
            }
        };
        if js_status.set_named_property("code", code_val).is_err() {
            send_default(&request);
            return;
        }
        js_args.push(js_status.into_unknown());
    }

    let call_result = js_fn.call(None, &js_args);

    if request.response_tx.is_none() {
        return;
    }

    let rcs_code = if userdata.has_rust_call_status {
        if let Some(js_status_unknown) = js_args.last() {
            if let Ok(js_status_obj) =
                unsafe { JsObject::from_raw(userdata.raw_env, js_status_unknown.raw()) }
            {
                js_status_obj
                    .get_named_property::<i32>("code")
                    .map(|c| c as i8)
                    .unwrap_or(request.rust_call_status_code)
            } else {
                request.rust_call_status_code
            }
        } else {
            request.rust_call_status_code
        }
    } else {
        0
    };

    let return_value = match call_result {
        Ok(js_ret) => {
            if matches!(userdata.ret_type, FfiTypeDesc::Void) {
                None
            } else {
                js_return_to_raw(env, &userdata.ret_type, js_ret)
            }
        }
        Err(_) => None,
    };

    if let Some(tx) = request.response_tx {
        let _ = tx.send(VTableCallResponse {
            return_value,
            rust_call_status_code: rcs_code,
        });
    }
}

/// Minimal RustCallStatus layout for reading/writing through VTable callbacks.
#[repr(C)]
struct RustCallStatusForVTable {
    code: i8,
    // error_buf follows but we don't need to touch it in the trampoline
}

/// Build a C struct (VTable) from a JS object.
///
/// For each field that is a `Callback(name)`:
/// 1. Look up the callback definition
/// 2. Get the JS function from the object property
/// 3. Create a long-lived trampoline (leaked) that handles return values and RustCallStatus
/// 4. Store the function pointer in the struct memory
///
/// Returns a `Vec<*const c_void>` where each element is a function pointer.
/// The Vec is leaked so the struct memory lives as long as the VTable is needed.
/// Returns a raw pointer to the struct data (pointer to first element).
pub fn build_vtable_struct(
    env: &Env,
    struct_def: &StructDef,
    js_obj: &JsObject,
    callback_defs: &HashMap<String, CallbackDef>,
    rb_from_bytes_ptr: *const c_void,
    rb_free_ptr: *const c_void,
) -> Result<*const c_void> {
    let field_count = struct_def.fields.len();
    let mut vtable_data: Vec<*const c_void> = Vec::with_capacity(field_count);

    for field in &struct_def.fields {
        match &field.field_type {
            FfiTypeDesc::Callback(cb_name) => {
                let cb_def = callback_defs.get(cb_name).ok_or_else(|| {
                    napi::Error::from_reason(format!(
                        "Unknown callback '{}' for struct field '{}'",
                        cb_name, field.name
                    ))
                })?;

                // Get the JS function from the object
                let js_fn_val: JsUnknown = js_obj.get_named_property(&field.name)?;
                let raw_fn_val = unsafe { js_fn_val.raw() };

                // Create a persistent reference to prevent GC
                let mut fn_ref: napi::sys::napi_ref = std::ptr::null_mut();
                let ref_status = unsafe {
                    napi::sys::napi_create_reference(env.raw(), raw_fn_val, 1, &mut fn_ref)
                };
                if ref_status != napi::sys::Status::napi_ok {
                    return Err(napi::Error::from_reason(format!(
                        "Failed to create reference for VTable field '{}'",
                        field.name
                    )));
                }

                // Build CIF for this callback:
                // declared args + optional RustCallStatus pointer -> ret type
                let mut cif_arg_types: Vec<Type> = cb_def.args.iter().map(ffi_type_for).collect();
                if cb_def.has_rust_call_status {
                    cif_arg_types.push(Type::pointer());
                }
                let cif_ret_type = ffi_type_for(&cb_def.ret);
                let cif = Cif::new(cif_arg_types, cif_ret_type);

                // Create userdata
                let userdata = Box::new(VTableTrampolineUserdata {
                    raw_env: env.raw(),
                    fn_ref,
                    arg_types: cb_def.args.clone(),
                    ret_type: cb_def.ret.clone(),
                    has_rust_call_status: cb_def.has_rust_call_status,
                    tsfn: None, // Will be set below
                    rb_from_bytes_ptr,
                    rb_free_ptr,
                });

                // Leak userdata to a raw pointer for stable address.
                // Do NOT create &'static ref yet — we still need to mutate tsfn.
                let userdata_ptr = Box::into_raw(userdata);

                // Create a no-op JS function for the TSF base. The TSF auto-calls its base
                // function with the callback's returned Vec<JsUnknown>. By using a no-op,
                // the auto-call is harmless — the real JS function is called manually in
                // vtable_tsfn_handler via fn_ref.
                let noop_fn =
                    env.create_function_from_closure("vtable_tsfn_noop", |_ctx| Ok(()))?;

                // TSF closure captures raw pointer, not &'static ref.
                // Cast to usize so the closure is Send (raw pointers aren't Send).
                let tsfn: ThreadsafeFunction<VTableCallRequest, ErrorStrategy::Fatal> = {
                    let ud_addr = userdata_ptr as usize;
                    noop_fn.create_threadsafe_function(
                        0,
                        move |ctx: napi::threadsafe_function::ThreadSafeCallContext<
                            VTableCallRequest,
                        >| {
                            let ud = unsafe { &*(ud_addr as *const VTableTrampolineUserdata) };
                            vtable_tsfn_handler(&ctx.env, ud, ctx.value);
                            Ok(Vec::<napi::JsUnknown>::new())
                        },
                    )?
                };

                // Unref so it doesn't keep the event loop alive
                let mut tsfn = tsfn;
                tsfn.unref(env)?;

                // Store TSF in userdata (still using raw pointer, before creating &'static ref)
                unsafe {
                    (*userdata_ptr).tsfn = Some(tsfn);
                }

                // NOW safe to create the &'static reference — all mutations are done
                let userdata_ref: &'static VTableTrampolineUserdata = unsafe { &*userdata_ptr };

                // Create closure
                let closure = Closure::new(cif, vtable_trampoline_callback, userdata_ref);

                // Extract function pointer
                let fn_ptr: *const c_void = *closure.code_ptr() as *const std::ffi::c_void;

                // Leak the closure so the function pointer stays valid
                std::mem::forget(closure);

                vtable_data.push(fn_ptr);
            }
            _ => {
                return Err(napi::Error::from_reason(format!(
                    "Unsupported struct field type for '{}': {:?}. Only Callback fields are supported in VTable structs.",
                    field.name, field.field_type
                )));
            }
        }
    }

    // Leak the Vec so the struct memory persists
    let ptr = vtable_data.as_ptr() as *const c_void;
    std::mem::forget(vtable_data);

    Ok(ptr)
}
