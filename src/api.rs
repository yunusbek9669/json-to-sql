use libc::c_char;
use std::ffi::{CStr, CString};
use serde_json::json;
use std::collections::HashMap;

use crate::parser;
use crate::generator;
use crate::info; // from info.rs

/// Parses a declarative JSON string and returns a parameterized SQL JSON result.
/// 
/// Returns a heap-allocated C string. Ownership is transferred to the caller.
/// The caller MUST free the string using `uaq_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn uaq_parse(json_input: *const c_char, whitelist_input: *const c_char, relations_input: *const c_char) -> *mut c_char {
    if json_input.is_null() {
        return create_error_result("Input is null");
    }

    let c_str = unsafe { CStr::from_ptr(json_input) };
    let json_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return create_error_result("Invalid UTF-8 in input"),
    };

    let parsed_json: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return create_error_result("Invalid JSON format"),
    };

    let whitelist_str = if whitelist_input.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(whitelist_input).to_str().ok() }
    };

    let relations_str = if relations_input.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(relations_input).to_str().ok() }
    };

    // Check for @info request
    if let Some(info_arr) = parsed_json.get("@info").and_then(|v| v.as_array()) {
        let result = info::process_info_request(info_arr, whitelist_str, relations_str);
        return encode_result(result);
    }

    let whitelist: Option<HashMap<String, serde_json::Value>> = if let Some(s) = whitelist_str {
        if s.trim().is_empty() {
             None
        } else {
             match serde_json::from_str(s) {
                 Ok(w) => Some(w),
                 Err(e) => return create_error_result(&format!("Whitelist Parse Error: {}", e)),
             }
        }
    } else {
        None
    };

    let relations: Option<HashMap<String, String>> = if let Some(s) = relations_str {
        if s.trim().is_empty() {
             None
        } else {
             match serde_json::from_str(s) {
                 Ok(r) => Some(r),
                 Err(e) => return create_error_result(&format!("Relations Parse Error: {}", e)),
             }
        }
    } else {
        None
    };

    let root_node = match parser::parse_json(json_str) {
        Ok(res) => res,
        Err(e) => return create_error_result(&format!("Parse Error: {}", e)),
    };

    let generator = generator::SqlGenerator::new(whitelist, relations);
    let sql_result = match generator.generate(root_node) {
        Ok(res) => res,
        Err(e) => return create_error_result(&format!("Generation Error: {}", e)),
    };

    let serialized = match serde_json::to_string(&sql_result) {
        Ok(s) => s,
        Err(e) => return create_error_result(&format!("Serialization Error: {}", e)),
    };

    CString::new(serialized).unwrap().into_raw()
}

/// Frees a string previously allocated by `uaq_parse`.
#[unsafe(no_mangle)]
pub extern "C" fn uaq_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = CString::from_raw(s);
        }
    }
}

pub(crate) fn create_error_result(msg: &str) -> *mut c_char {
    let err_json = json!({
        "isOk": false,
        "sql": null,
        "params": null,
        "structure": null,
        "message": msg
    });
    encode_result(err_json)
}

fn encode_result(val: serde_json::Value) -> *mut c_char {
    let s = serde_json::to_string(&val).unwrap();
    CString::new(s).unwrap().into_raw()
}
