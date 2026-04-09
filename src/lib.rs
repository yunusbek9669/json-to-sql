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

    let parsed_json: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return create_error_result("Invalid JSON format"),
    };

    if let Some(info_arr) = parsed_json.get("@info").and_then(|v| v.as_array()) {
        let is_tables = info_arr.iter().any(|v| v.as_str() == Some("@tables"));
        let is_relations = info_arr.iter().any(|v| v.as_str() == Some("@relations"));
        
        let mut info_result = serde_json::Map::new();
        let mut included_relations = String::from("[]");

        if is_relations && !relations_input.is_null() {
            let rel_str = unsafe { CStr::from_ptr(relations_input).to_str().unwrap_or("{}") };
            let rel_map: HashMap<String, String> = serde_json::from_str(rel_str).unwrap_or_default();
            let keys: Vec<String> = rel_map.keys().cloned().collect();
            included_relations = serde_json::to_string(&keys).unwrap_or_else(|_| "[]".to_string());
            info_result.insert("relations".to_string(), json!(keys));
        }

        if is_tables && !whitelist_input.is_null() {
            let whitelist_str = unsafe { CStr::from_ptr(whitelist_input).to_str().unwrap_or("{}") };
            let wl_escaped = whitelist_str.replace("'", "''");
            let rel_escaped = included_relations.replace("'", "''");
            let sql_query = format!(r#"WITH input_json AS (
    SELECT '{}'::jsonb AS data
),
parsed_tables AS (
    SELECT 
        split_part(key, ':', 1) AS table_name,
        COALESCE(NULLIF(split_part(key, ':', 2), ''), split_part(key, ':', 1)) AS table_alias,
        value AS col_data
    FROM input_json, jsonb_each(data)
),
parsed_columns AS (
    SELECT 
        pt.table_name,
        pt.table_alias,
        obj.value AS real_col,
        CASE 
            WHEN obj.key ~ '^[0-9]+$' THEN obj.value
            ELSE obj.key
        END AS col_alias
    FROM parsed_tables pt
    CROSS JOIN LATERAL jsonb_each_text(
        CASE WHEN jsonb_typeof(pt.col_data) = 'object' THEN pt.col_data ELSE '{{}}'::jsonb END
    ) obj
    
    UNION ALL

    SELECT 
        pt.table_name,
        pt.table_alias,
        arr.value AS real_col,
        arr.value AS col_alias
    FROM parsed_tables pt
    CROSS JOIN LATERAL jsonb_array_elements_text(
        CASE WHEN jsonb_typeof(pt.col_data) = 'array' THEN pt.col_data ELSE '[]'::jsonb END
    ) arr
),
joined_schema AS (
    SELECT 
        pc.table_alias,
        CASE 
            WHEN pc.real_col = '*' THEN COALESCE(c.column_name, 'TABLE_NOT_FOUND') 
            ELSE pc.col_alias 
        END AS final_col_alias,
        CASE 
            WHEN pc.real_col ~* '[\(\s]|CASE|WHEN|END' THEN 'text' 
            ELSE COALESCE(c.data_type, 'COLUMN_NOT_FOUND_IN_DB') 
        END AS data_type
    FROM parsed_columns pc
    LEFT JOIN information_schema.columns c 
      ON c.table_name = pc.table_name 
      AND c.table_schema = 'public'
      AND (pc.real_col = '*' OR c.column_name = pc.real_col)
),
tables_json AS (
    SELECT jsonb_object_agg(table_alias, cols) AS tables_obj
    FROM (
        SELECT table_alias, jsonb_object_agg(final_col_alias, data_type) AS cols
        FROM joined_schema
        GROUP BY table_alias
    ) subquery
)
SELECT jsonb_build_object(
    'tables', COALESCE((SELECT tables_obj FROM tables_json), '{{}}'::jsonb),
    'relations', '{}'::jsonb
) AS result;"#, wl_escaped, rel_escaped);
            info_result.insert("sql".to_string(), json!(sql_query));
        }

        let mut result = json!({
            "isOk": true,
            "sql": null,
            "params": null,
            "structure": info_result.clone(),
            "message": "info"
        });
        
        if let Some(sql_val) = info_result.get("sql") {
            result["sql"] = sql_val.clone();
        }
        
        let serialized = serde_json::to_string(&result).unwrap();
        return CString::new(serialized).unwrap().into_raw();
    }

    let whitelist_str = if whitelist_input.is_null() {
        None
    } else {
        unsafe { CStr::from_ptr(whitelist_input).to_str().ok() }
    };
    
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
        wl.insert("employee:emp".to_string(), json!(["id", "last_name", "first_name", "status", "organization_id"]));
        wl.insert("structure_organization:org".to_string(), json!(["*"]));

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
        wl.insert("employee:emp".to_string(), json!(["*"]));

        let root = parser::parse_json(json_input).expect("Should parse");
        let gen_inst = generator::SqlGenerator::new(Some(wl), None);
        let result = gen_inst.generate(root);
        
        assert!(result.is_err(), "Should reject raw table name when alias exists");
        let err = result.unwrap_err();
        assert!(err.contains("does not exist"), "Error should match whitelist format: {}", err);
        println!("Enforcement error (expected): {}", err);
    }

    #[test]
    fn test_auto_path_resolution() {
        // Frontend writes org/inner_org directly under emp — no nested structure needed!
        let json_input = r#"{
            "@source": "emp[status: 1, $limit: 2]",
            "@fields": {
                "id": "id",
                "full_name": "CONCAT(last_name, ' ', first_name)"
            },
            "viloyat_boshqarma": {
                "@source": "org[status: 1]",
                "@fields": { "name": "name_uz" }
            },
            "tuman_boshqarma": {
                "@source": "inner_org[status: 1]",
                "@fields": { "name": "name_uz" }
            }
        }"#;

        let mut wl = std::collections::HashMap::new();
        wl.insert("employee:emp".to_string(), json!(["*"]));
        wl.insert("employee_department_staff_position:dept".to_string(), json!(["*"]));
        wl.insert("shtat_department_basic:dept_basic".to_string(), json!(["*"]));
        wl.insert("structure_organization:org".to_string(), json!(["*"]));
        wl.insert("structure_organization:inner_org".to_string(), json!(["*"]));

        let mut rels = std::collections::HashMap::new();
        rels.insert("emp->dept".to_string(), "INNER JOIN @table ON @1.id = @2.employee_id AND @2.status = 1".to_string());
        rels.insert("dept->dept_basic".to_string(), "INNER JOIN @table ON @1.department_basic_id = @2.id".to_string());
        rels.insert("dept_basic<->org".to_string(), "INNER JOIN @table ON @1.organization_id = @2.id".to_string());
        rels.insert("dept_basic<->inner_org".to_string(), "INNER JOIN @table ON @1.command_organization_id = @2.id".to_string());

        let root = parser::parse_json(json_input).expect("Should parse");
        let gen_inst = generator::SqlGenerator::new(Some(wl), Some(rels));
        let result = gen_inst.generate(root).expect("Auto-path should work");

        let sql_str = result.sql.as_ref().unwrap();
        // Engine should auto-discover path: emp → dept → dept_basic → org/inner_org
        assert!(sql_str.contains("FROM employee AS emp"), "Root table");
        assert!(sql_str.contains("INNER JOIN employee_department_staff_position AS dept"), "Auto-joined intermediate: dept");
        assert!(sql_str.contains("INNER JOIN shtat_department_basic AS dept_basic"), "Auto-joined intermediate: dept_basic");
        assert!(sql_str.contains("INNER JOIN structure_organization AS org"), "Target: org");
        assert!(sql_str.contains("INNER JOIN structure_organization AS inner_org"), "Target: inner_org");

        println!("Auto-Path SQL:\n{}", serde_json::to_string_pretty(&result).unwrap());
    }

    #[test]
    fn test_info_endpoint() {
        let json_input = "{\"@info\": [\"@tables\", \"@relations\"]}\0".as_ptr() as *const c_char;
        let whitelist_input = "{\"employee:emp\": {\"unique\": \"id\", \"full_name\": \"CONCAT(name)\"}, \"org\": [\"*\"]}\0".as_ptr() as *const c_char;
        let relations_input = "{\"emp->org\": \"JOIN\", \"org->dept\": \"JOIN\"}\0".as_ptr() as *const c_char;

        let result_ptr = uaq_parse(json_input, whitelist_input, relations_input);
        assert!(!result_ptr.is_null());

        let c_str = unsafe { CStr::from_ptr(result_ptr) };
        let result_str = c_str.to_str().unwrap();
        println!("Info Result: {}", result_str);
        
        let result_json: serde_json::Value = serde_json::from_str(result_str).unwrap();
        assert_eq!(result_json["isOk"], true);
        assert_eq!(result_json["message"], "info");
        
        let structure = result_json["structure"].as_object().unwrap();
        assert!(structure.contains_key("sql"));
        assert!(structure.contains_key("relations"));
        
        // Also check root sql property
        let sql = result_json["sql"].as_str().unwrap();
        assert!(sql.contains("WITH input_json AS"));
        assert!(sql.contains("CONCAT(name)"));
        
        let rels = structure["relations"].as_array().unwrap();
        assert_eq!(rels.len(), 2);
        
        uaq_free_string(result_ptr);
    }

    #[test]
    fn test_user_complex_mapping() {
        let json_input = concat!(r#"{
          "@source": "emp[status: 1, id: 1000..2145, $limit: 20, $order: id DESC]",
          "@fields": {
            "id": "id",
            "full_name": "CONCAT(last_name, ' ', first_name)",
            "passport": "jshshir",
            "birthDay": "TO_CHAR(TO_TIMESTAMP(birthday), 'DD.MM.YYYY')"
          },
          "0": {
              "@source": "org[red: 1]",
              "@flatten": true,
              "@fields": {
                  "viloyat boshqarma": "name"
              }
          }
        }"#, "\0").as_ptr() as *const c_char;

        let whitelist_input = concat!(r#"{
          "employee:emp": ["*"],
          "structure_organization:org": {
            "unique": "id",
            "name": "name_uz",
            "red": "status"
          },
          "structure_organization:inner_org": ["id", "name_uz", "status"],
          "employee_department_staff_position:department_staff_position": ["*"],
          "shtat_department_basic:department_basic": ["*"]
        }"#, "\0").as_ptr() as *const c_char;

        let relations_input = concat!(r#"{
          "emp->department_staff_position": "INNER JOIN @table ON @1.id = @2.employee_id AND @2.status = 1",
          "department_staff_position->department_basic": "INNER JOIN @table ON @1.department_basic_id = @2.id",
          "department_basic<->org": "INNER JOIN @table ON @1.organization_id = @2.id AND @1.status = 1",
          "department_basic<->inner_org": "INNER JOIN @table ON @1.command_organization_id = @2.id AND @1.status = 1"
        }"#, "\0").as_ptr() as *const c_char;

        let result_ptr = uaq_parse(json_input, whitelist_input, relations_input);
        assert!(!result_ptr.is_null());

        let c_str = unsafe { CStr::from_ptr(result_ptr) };
        let result_str = c_str.to_str().unwrap();
        println!("User Mapping Result:\n{}", result_str);
        
        let result_json: serde_json::Value = serde_json::from_str(result_str).unwrap();
        assert_eq!(result_json["isOk"], true);
        
        let sql = result_json["sql"].as_str().unwrap();
        assert!(sql.contains("CONCAT(emp.last_name, ' ', emp.first_name)"));
        // `name_uz` should be prefixed with org
        assert!(sql.contains("org.name_uz"));
        // Since it's aliased natively
        
        uaq_free_string(result_ptr);
    }
}
