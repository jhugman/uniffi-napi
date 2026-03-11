use std::collections::HashMap;
use std::ffi::c_void;

use libffi::low;
use libffi::middle::{Cif, Closure, Type};
use napi::{Env, JsObject, JsUnknown, NapiRaw, NapiValue, Result};

use crate::callback::CallbackDef;
use crate::cif::ffi_type_for;
use crate::ffi_type::FfiTypeDesc;

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
    let env = Env::from_raw(userdata.raw_env);

    // Get the JS function from the persistent reference
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

    // Total C args = declared args + (optional RustCallStatus pointer)
    let declared_count = userdata.arg_types.len();

    let mut js_args: Vec<napi::JsUnknown> = Vec::with_capacity(declared_count + 1);

    // Convert declared args to JS
    for (i, desc) in userdata.arg_types.iter().enumerate() {
        let arg_ptr = *args.add(i);
        let js_val = match c_arg_to_js_vtable(&env, desc, arg_ptr) {
            Ok(v) => v,
            Err(_) => return,
        };
        js_args.push(js_val);
    }

    // If hasRustCallStatus, the last C arg is a pointer to RustCallStatus.
    // Create a JS { code: number } object and pass it to the JS function.
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

    // Call the JS function
    let call_result = js_fn.call(None, &js_args);

    // Write back RustCallStatus if applicable
    if userdata.has_rust_call_status && !status_ptr.is_null() {
        // Read back the code from the last JS arg (the status object)
        // The JS function may have modified callStatus.code
        if let Some(js_status_unknown) = js_args.last() {
            if let Ok(js_status_obj) = JsObject::from_raw(userdata.raw_env, js_status_unknown.raw())
            {
                if let Ok(code_val) = js_status_obj.get_named_property::<i32>("code") {
                    (*status_ptr).code = code_val as i8;
                }
            }
        }
    }

    // Write return value to the result buffer.
    // For libffi closures, the result buffer is at least ffi_arg sized.
    // For integer types smaller than ffi_arg, we write as ffi_arg to ensure
    // correct behavior across platforms.
    if let Ok(js_ret) = call_result {
        write_return_value(result, &userdata.ret_type, userdata.raw_env, js_ret);
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
        _ => {} // Unsupported return types silently ignored
    }
}

/// Minimal RustCallStatus layout for reading/writing through VTable callbacks.
#[repr(C)]
struct RustCallStatusForVTable {
    code: i8,
    // error_buf follows but we don't need to touch it in the trampoline
}

/// Convert a C argument to a JS value (for VTable trampoline use).
unsafe fn c_arg_to_js_vtable(
    env: &Env,
    desc: &FfiTypeDesc,
    arg_ptr: *const c_void,
) -> Result<napi::JsUnknown> {
    match desc {
        FfiTypeDesc::UInt8 => {
            let v = *(arg_ptr as *const u8);
            Ok(env.create_uint32(v as u32)?.into_unknown())
        }
        FfiTypeDesc::Int8 => {
            let v = *(arg_ptr as *const i8);
            Ok(env.create_int32(v as i32)?.into_unknown())
        }
        FfiTypeDesc::UInt16 => {
            let v = *(arg_ptr as *const u16);
            Ok(env.create_uint32(v as u32)?.into_unknown())
        }
        FfiTypeDesc::Int16 => {
            let v = *(arg_ptr as *const i16);
            Ok(env.create_int32(v as i32)?.into_unknown())
        }
        FfiTypeDesc::UInt32 => {
            let v = *(arg_ptr as *const u32);
            Ok(env.create_uint32(v)?.into_unknown())
        }
        FfiTypeDesc::Int32 => {
            let v = *(arg_ptr as *const i32);
            Ok(env.create_int32(v)?.into_unknown())
        }
        FfiTypeDesc::UInt64 | FfiTypeDesc::Handle => {
            let v = *(arg_ptr as *const u64);
            Ok(env.create_bigint_from_u64(v)?.into_unknown()?)
        }
        FfiTypeDesc::Int64 => {
            let v = *(arg_ptr as *const i64);
            Ok(env.create_bigint_from_i64(v)?.into_unknown()?)
        }
        FfiTypeDesc::Float32 => {
            let v = *(arg_ptr as *const f32);
            Ok(env.create_double(v as f64)?.into_unknown())
        }
        FfiTypeDesc::Float64 => {
            let v = *(arg_ptr as *const f64);
            Ok(env.create_double(v)?.into_unknown())
        }
        _ => Err(napi::Error::from_reason(format!(
            "Unsupported VTable callback arg type: {:?}",
            desc
        ))),
    }
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
                });

                // Leak userdata for stable address
                let userdata_ptr = Box::into_raw(userdata);
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
