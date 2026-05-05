use libc::c_char;
use std::ffi::{CStr, CString};
use serde_json::json;
use std::collections::HashMap;

use crate::parser;
use crate::generator;
use crate::info;
use crate::operation;
use crate::format::process_files_in_json;

use indexmap::IndexMap;

/// Parses a declarative JSON string and returns a parameterized SQL JSON result.
/// 
/// Returns a heap-allocated C string. Ownership is transferred to the caller.
/// The caller MUST free the string using `uaq_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn uaq_parse(
    json_input: *const c_char, 
    whitelist_input: *const c_char, 
    relations_input: *const c_char,
    macros_input: *const c_char
) -> *mut c_char {
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

    let macros_str = if macros_input.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(macros_input).to_str().ok() }
    };

    // Check for @info request
    if let Some(info_arr) = parsed_json.get("@info").and_then(|v| v.as_array()) {
        let result = info::process_info_request(info_arr, whitelist_str, relations_str, macros_str);
        return encode_result(result);
    }

    let whitelist: Option<IndexMap<String, serde_json::Value>> = if let Some(s) = whitelist_str {
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

    let macros: Option<IndexMap<String, serde_json::Value>> = if let Some(s) = macros_str {
        let cleaned = s.trim_matches(|c: char| c.is_whitespace() || c == '\0' || c == '"' || c == '\'');
        if cleaned.is_empty() || cleaned == "null" || cleaned == "[]" || cleaned == "{}" {
            None
        } else {
            // Silently fall back to None if the backend gave invalid JSON for macros, 
            // since macros are entirely optional. Or we can just log/error. Let's return error if it's definitively garbled.
            match serde_json::from_str(s) {
                Ok(m) => Some(m),
                Err(_) => {
                    // Try parsing cleaned version
                    match serde_json::from_str(cleaned) {
                        Ok(mc) => Some(mc),
                        Err(_) => None // Optional: just gracefully return None rather than throwing a hard error if left empty
                    }
                }
            }
        }
    } else {
        None
    };

    // Check for @operation request (after whitelist is parsed)
    if let Some(op_val) = parsed_json.get("@operation") {
        if op_val.is_object() || op_val.is_array() {
            let result = operation::process_operation(op_val, whitelist);
            return encode_result(result);
        }
    }

    let root_node = match parser::parse_json(json_str, macros.as_ref()) {
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
        "message": msg
    });
    encode_result(err_json)
}

/// A utility function to convert string paths into base64 embedded files.
/// `json_result` should be the actual data from the database.
/// `root_files_path` absolute directory path for files (e.g. "/var/www/uploads")
/// `trigger_prefix` an exact string indicating which strings to process (e.g. "/web/uploads/")
#[unsafe(no_mangle)]
pub extern "C" fn uaq_inject_base64_files(
    json_result: *const c_char,
    root_files_path: *const c_char,
    trigger_prefix: *const c_char
) -> *mut c_char {
    if json_result.is_null() {
        return create_error_result("uaq_inject_base64_files: json_result is null — pass the DB query result as a JSON string");
    }
    if root_files_path.is_null() {
        return create_error_result("uaq_inject_base64_files: root_files_path is null");
    }
    if trigger_prefix.is_null() {
        return create_error_result("uaq_inject_base64_files: trigger_prefix is null");
    }

    let c_json = unsafe { CStr::from_ptr(json_result) };
    let json_str = match c_json.to_str() {
        Ok(s) => s,
        Err(_) => return create_error_result("uaq_inject_base64_files: json_result is not valid UTF-8"),
    };

    if json_str.is_empty() {
        return create_error_result("uaq_inject_base64_files: json_result is an empty string — DB query likely returned no rows (false/null)");
    }

    let mut parsed_json: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => return create_error_result(&format!(
            "uaq_inject_base64_files: json_result is not valid JSON — {}. Received: {}",
            e,
            if json_str.len() > 80 { &json_str[..80] } else { json_str }
        )),
    };

    if !parsed_json.is_object() && !parsed_json.is_array() {
        return create_error_result(&format!(
            "uaq_inject_base64_files: json_result must be a JSON object or array, got: {}",
            match &parsed_json {
                serde_json::Value::Null    => "null",
                serde_json::Value::Bool(_) => "boolean",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                _ => "unknown",
            }
        ));
    }

    let c_root = unsafe { CStr::from_ptr(root_files_path) };
    let root_str = match c_root.to_str() {
        Ok(s) => s,
        Err(_) => return create_error_result("uaq_inject_base64_files: root_files_path is not valid UTF-8"),
    };

    let c_trigger = unsafe { CStr::from_ptr(trigger_prefix) };
    let trigger_str = match c_trigger.to_str() {
        Ok(s) => s,
        Err(_) => return create_error_result("uaq_inject_base64_files: trigger_prefix is not valid UTF-8"),
    };

    // Traverse the JSON and replace strings starting with trigger_str
    process_files_in_json(&mut parsed_json, root_str, trigger_str);

    let serialized = match serde_json::to_string(&parsed_json) {
        Ok(s) => s,
        Err(e) => return create_error_result(&format!("Serialization Error: {}", e)),
    };

    CString::new(serialized).unwrap().into_raw()
}

fn encode_result(val: serde_json::Value) -> *mut c_char {
    let s = serde_json::to_string(&val).unwrap();
    CString::new(s).unwrap().into_raw()
}
