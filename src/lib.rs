pub mod models;
pub mod parser;
pub mod generator;
pub mod guard;

use libc::c_char;
use std::ffi::{CStr, CString};
use serde_json::json;

use std::collections::HashMap;

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

    let whitelist_str = if whitelist_input.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(whitelist_input).to_str().ok() }
    };
    
    let whitelist: Option<HashMap<String, Vec<String>>> = if let Some(s) = whitelist_str {
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

    let relations_str = if relations_input.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(relations_input).to_str().ok() }
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

fn create_error_result(msg: &str) -> *mut c_char {
    let err_json = json!({
        "isOk": false,
        "sql": null,
        "params": null,
        "structure": null,
        "message": msg
    });
    let s = serde_json::to_string(&err_json).unwrap();
    CString::new(s).unwrap().into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_format() {
        // New compact format: no @data/@config wrappers
        let json_input = r#"{
            "@source": "personal[status: 'active', age: 25..45, $limit: 15, $order: personal.id DESC]",
            "@fields": {
                "id": "id",
                "full_name": "CONCAT(last_name_latin, ' ', first_name_latin)",
                "passport": "jshshir"
            },
            "organization": {
                "@source": "org",
                "@fields": {
                    "name": "name_uz",
                    "code": "code"
                }
            },
            "position_info": {
                "@source": "pos[rank_id: in (1, 2, 3)]",
                "@flatten": true,
                "@fields": {
                    "title": "name_latin",
                    "is_military": "is_military_rank"
                }
            }
        }"#;

        let root = parser::parse_json(json_input).expect("Should parse");
        
        // Verify $limit and $order were parsed
        assert_eq!(root.source.as_ref().unwrap().limit, Some(15));
        assert_eq!(root.source.as_ref().unwrap().order.as_deref(), Some("personal.id DESC"));
        
        let mut rels = std::collections::HashMap::new();
        rels.insert("personal<->org".to_string(), "INNER JOIN @table ON @1.org_id = @2.id".to_string());
        rels.insert("personal<->pos".to_string(), "LEFT JOIN @table ON @1.pos_id = @2.id".to_string());
        let gen_inst = generator::SqlGenerator::new(None, Some(rels));
        let result = gen_inst.generate(root).expect("Should generate");

        let sql_str = result.sql.as_ref().unwrap();
        assert!(sql_str.contains("SELECT COALESCE(json_agg(t.uaq_data), '[]'::json)"));
        assert!(sql_str.contains("SELECT json_build_object("));
        assert!(sql_str.contains("'id', personal.id"));
        assert!(sql_str.contains("CONCAT(personal.last_name_latin, ' ', personal.first_name_latin)"));
        assert!(sql_str.contains("INNER JOIN org ON personal.org_id = org.id"));
        assert!(sql_str.contains("LIMIT 15"));
        assert!(sql_str.contains("ORDER BY personal.id DESC"));
        assert!(result.params.as_ref().unwrap().len() > 0);
        
        let serialized = serde_json::to_string_pretty(&result).unwrap();
        println!("Generated SQL:\n{}", serialized);
    }

    #[test]
    fn test_legacy_format() {
        // Legacy format with @data/@config still works
        let json_input = r#"{
            "@data": {
                "@source": "employee[status: 1, id: 1..45]",
                "@fields": {
                    "id": "id",
                    "full_name": "CONCAT(last_name, ' ', first_name)"
                }
            },
            "@config": {
                "limit": 2,
                "order": "employee.id DESC"
            }
        }"#;

        let root = parser::parse_json(json_input).expect("Should parse legacy");
        assert_eq!(root.source.as_ref().unwrap().limit, Some(2));
        assert_eq!(root.source.as_ref().unwrap().order.as_deref(), Some("employee.id DESC"));
        
        let gen_inst = generator::SqlGenerator::new(None, None);
        let result = gen_inst.generate(root).expect("Should generate");
        let sql_str = result.sql.as_ref().unwrap();
        assert!(sql_str.contains("LIMIT 2"));
        assert!(sql_str.contains("ORDER BY employee.id DESC"));
        
        println!("Legacy SQL:\n{}", serde_json::to_string_pretty(&result).unwrap());
    }

    #[test]
    fn test_alias_format() {
        // Frontend uses aliases defined in whitelist
        let json_input = r#"{
            "@source": "emp[status: 1, $limit: 5]",
            "@fields": {
                "id": "id",
                "full_name": "CONCAT(last_name, ' ', first_name)"
            },
            "boshqarma": {
                "@source": "org[status: 1]",
                "@fields": {
                    "name": "name_uz"
                }
            }
        }"#;

        // Whitelist with aliases: "real_table:alias"
        let mut wl = std::collections::HashMap::new();
        wl.insert("employee:emp".to_string(), vec!["id".to_string(), "last_name".to_string(), "first_name".to_string(), "status".to_string(), "organization_id".to_string()]);
        wl.insert("structure_organization:org".to_string(), vec!["*".to_string()]);

        // Relations use ALIAS names in keys
        let mut rels = std::collections::HashMap::new();
        rels.insert("emp<->org".to_string(), "INNER JOIN @table ON @1.organization_id = @2.id".to_string());

        let root = parser::parse_json(json_input).expect("Should parse alias format");
        let gen_inst = generator::SqlGenerator::new(Some(wl), Some(rels));
        let result = gen_inst.generate(root).expect("Should generate with aliases");

        let sql_str = result.sql.as_ref().unwrap();
        // SQL uses REAL table in FROM/JOIN, alias as SQL alias
        assert!(sql_str.contains("FROM employee AS emp"), "Should use FROM real AS alias");
        assert!(sql_str.contains("INNER JOIN structure_organization AS org ON emp.organization_id = org.id"), "Should resolve alias to real join with AS alias");
        assert!(sql_str.contains("'id', emp.id"), "Auto-prefix should use alias");
        assert!(sql_str.contains("LIMIT 5"));

        println!("Alias SQL:\n{}", serde_json::to_string_pretty(&result).unwrap());
    }

    #[test]
    fn test_alias_enforcement() {
        // Frontend tries to use real table name when alias is defined → must fail
        let json_input = r#"{
            "@source": "employee[status: 1]",
            "@fields": { "id": "id" }
        }"#;

        let mut wl = std::collections::HashMap::new();
        wl.insert("employee:emp".to_string(), vec!["*".to_string()]);

        let root = parser::parse_json(json_input).expect("Should parse");
        let gen_inst = generator::SqlGenerator::new(Some(wl), None);
        let result = gen_inst.generate(root);
        
        assert!(result.is_err(), "Should reject raw table name when alias exists");
        let err = result.unwrap_err();
        assert!(err.contains("is strictly prohibited by whitelist"), "Error should match whitelist format: {}", err);
        println!("Enforcement error (expected): {}", err);
    }
}
