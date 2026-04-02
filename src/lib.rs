pub mod models;
pub mod parser;
pub mod generator;
pub mod guard;

use libc::c_char;
use std::ffi::{CStr, CString};
use serde_json::json;

/// Parses a declarative JSON string and returns a parameterized SQL JSON result.
/// 
/// Returns a heap-allocated C string. Ownership is transferred to the caller.
/// The caller MUST free the string using `uaq_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn uaq_parse(json_input: *const c_char) -> *mut c_char {
    if json_input.is_null() {
        return create_error_result("Input is null");
    }

    let c_str = unsafe { CStr::from_ptr(json_input) };
    let json_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return create_error_result("Invalid UTF-8 in input"),
    };

    let parse_result = match parser::parse_json(json_str) {
        Ok(res) => res,
        Err(e) => return create_error_result(&format!("Parse Error: {}", e)),
    };

    let generator = generator::SqlGenerator::new();
    let sql_result = match generator.generate(parse_result) {
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

fn create_error_result(msg: &str) -> *mut c_char {
    let err_json = json!({
        "error": true,
        "message": msg
    });
    let s = serde_json::to_string(&err_json).unwrap();
    CString::new(s).unwrap().into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_employee_query() {
        let json_input = r#"{
            "employee": {
                "@source": "personal[status: 'active', age: 25..45]",
                "@fields": {
                    "id": "id",
                    "full_name": "CONCAT(last_name_latin, ' ', first_name_latin)",
                    "passport": "jshshir"
                },
                "organization": {
                    "@source": "org",
                    "@join": "INNER JOIN org ON personal.org_id = org.id",
                    "@fields": {
                        "name": "name_uz",
                        "code": "code"
                    }
                },
                "position_info": {
                    "@source": "pos[rank_id: in (1, 2, 3)]",
                    "@join": "LEFT JOIN pos ON personal.pos_id = pos.id",
                    "@fields": {
                        "title": "name_latin",
                        "is_military": "is_military_rank"
                    }
                }
            },
            "@config": {
                "limit": 15,
                "order": "personal.id DESC"
            }
        }"#;

        let root = parser::parse_json(json_input).expect("Should parse");
        let gen_inst = generator::SqlGenerator::new();
        let result = gen_inst.generate(root).expect("Should generate");

        assert!(result.sql.contains("SELECT"));
        assert!(result.sql.contains("employee_id"));
        assert!(result.sql.contains("employee_full_name"));
        assert!(result.sql.contains("INNER JOIN org ON personal.org_id = org.id"));
        assert!(result.params.len() > 0);
        
        let serialized = serde_json::to_string_pretty(&result).unwrap();
        println!("Generated SQL Setup Form:\n{}", serialized);
    }
}
