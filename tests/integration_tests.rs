use libc::c_char;
use std::ffi::CStr;
use serde_json::json;
use json_to_sql::parser;
use json_to_sql::generator;
use json_to_sql::{uaq_parse, uaq_free_string};

#[test]
fn test_compact_format() {
    // New compact format: no @data/@config wrappers
    let json_input = r#"{
        "@data[]": {
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
        }
    }"#;

    let root = parser::parse_json(json_input, None).expect("Should parse");
    
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
fn test_alias_format() {
    // Frontend uses aliases defined in whitelist
    let json_input = r#"{
        "@data[]": {
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
        }
    }"#;

    // Whitelist with aliases: "real_table:alias"
    let mut wl = indexmap::IndexMap::new();
    wl.insert("employee:emp".to_string(), json!(["id", "last_name", "first_name", "status", "organization_id"]));
    wl.insert("structure_organization:org".to_string(), json!(["*"]));

    // Relations use ALIAS names in keys
    let mut rels = std::collections::HashMap::new();
    rels.insert("emp<->org".to_string(), "INNER JOIN @table ON @1.organization_id = @2.id".to_string());

    let root = parser::parse_json(json_input, None).expect("Should parse alias format");
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
        "@data": {
            "@source": "employee[status: 1]",
            "@fields": { "id": "id" }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert("employee:emp".to_string(), json!(["*"]));

    let root = parser::parse_json(json_input, None).expect("Should parse");
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
        "@data[]": {
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
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
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

    let root = parser::parse_json(json_input, None).expect("Should parse");
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

    let result_ptr = uaq_parse(json_input, whitelist_input, relations_input, std::ptr::null());
    assert!(!result_ptr.is_null());

    let c_str = unsafe { CStr::from_ptr(result_ptr) };
    let result_str = c_str.to_str().unwrap();
    println!("Info Result: {}", result_str);
    
    let result_json: serde_json::Value = serde_json::from_str(result_str).unwrap();
    assert_eq!(result_json["isOk"], true);
    assert_eq!(result_json["message"], "info");
    
    // sql and relations are now at root level, not nested in structure
    let sql = result_json["sql"].as_str().unwrap();
    assert!(sql.contains("WITH input_json AS"));
    assert!(sql.contains("CONCAT(name)"));
    
    let rels = result_json["relations"].as_array().unwrap();
    assert_eq!(rels.len(), 2);
    
    uaq_free_string(result_ptr);
}

#[test]
fn test_user_complex_mapping() {
    let json_input = concat!(r#"{
      "@data[]": {
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

    let result_ptr = uaq_parse(json_input, whitelist_input, relations_input, std::ptr::null());
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

#[test]
fn test_parents_cte_generation() {
    let json_input = r#"{
        "@data[]": {
            "@source": "emp[status: 1, $limit: 5]",
            "@fields": { "id": "id", "full_name": "full_name" },
            "department": {
                "@source": "departmentBasic",
                "@fields": {
                    "id": "id",
                    "dep_path": "parents(parent_id, id, [name])",
                    "dep_str": "parents(parent_id, id, name)"
                }
            },
            "education_data": {
                "@source": "education",
                "@fields": {
                    "edu_count": "count(*)",
                    "max_end_year": "max(end_year)",
                    "active_edu": "count([status: 1])"
                }
            }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert("employee:emp".to_string(), json!({"full_name": "CONCAT(last_name, ' ', first_name)", "id": "id", "status": "status"}));
    wl.insert("shtat_department_basic:departmentBasic".to_string(), json!(["id", "parent_id", "name"]));
    wl.insert("employee_education:education".to_string(), json!(["id", "status", "end_year", "employee_id"]));

    let mut rels = std::collections::HashMap::new();
    rels.insert("emp<->departmentBasic".to_string(), "INNER JOIN @table ON @1.department_basic_id = @2.id".to_string());
    rels.insert("emp->education".to_string(), "LEFT JOIN @table ON @1.id = @2.employee_id".to_string());

    let root = parser::parse_json(json_input, None).expect("parse");
    let sql_gen = generator::SqlGenerator::new(Some(wl), Some(rels));
    let result = sql_gen.generate(root).expect("generate");

    let sql = result.sql.as_ref().unwrap();
    println!("parents() SQL:\n{}", sql);

    // LATERAL JOIN used (not scalar subquery) — guarantees per-row evaluation of the CTE
    assert!(sql.contains("LEFT JOIN LATERAL"), "Must use LATERAL JOIN for parents()");
    // CTE starts from the CURRENT node (base.id = outer_alias.id)
    assert!(sql.contains("departmentBasic_base.id = departmentBasic.id"), "Base case must start from current node");
    // Canonical form: CTE ref first in recursive FROM, then JOIN physical table
    assert!(sql.contains("FROM departmentBasic_tree"), "Canonical recursive form: CTE ref first");
    assert!(sql.contains("JOIN shtat_department_basic AS departmentBasic_r ON departmentBasic_r.id = departmentBasic_tree.parent_id"), "Recursive must climb to parent");
    // Explicit NULL termination at root
    assert!(sql.contains("departmentBasic_tree.parent_id IS NOT NULL"), "Must terminate at root via IS NOT NULL");
    // Depth limit
    assert!(sql.contains("_depth < 50"), "Depth limit must be present (default = 50)");
    // Root-first ordering
    assert!(sql.contains("ORDER BY _depth DESC"), "Must order root-first");
    // Each parents() call gets its own unique LATERAL alias → no naming conflicts
    assert!(sql.contains("_plat2.result"), "dep_path should reference lateral alias");
    assert!(sql.contains("_plat3.result"), "dep_str should reference a different lateral alias");
    // education_data: no JOIN in main query (all-aggregate fields → skip_join)
    assert!(!sql.contains("LEFT JOIN employee_education"), "education JOIN must be skipped");
    assert!(sql.contains("SELECT COUNT(*)"), "COUNT subquery must be generated");
    assert!(sql.contains("SELECT MAX("), "MAX subquery must be generated");
}

#[test]
fn test_parents_custom_key_syntax() {
    let json_input = r#"{
        "@data[]": {
            "@source": "emp[status: 1, $limit: 3]",
            "@fields": { "id": "id" },
            "department": {
                "@source": "departmentBasic",
                "@fields": {
                    "id": "id",
                    "dep_path_obj": "parents(parent_id, id, {nn:name})",
                    "dep_path_multi": "parents(parent_id, id, {title:name, key:id})",
                    "dep_path_arr": "parents(parent_id, id, [name])",
                    "dep_str": "parents(parent_id, id, name)"
                }
            }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert("employee:emp".to_string(), json!({"id":"id","status":"status"}));
    wl.insert("shtat_department_basic:departmentBasic".to_string(), json!(["id","parent_id","name"]));

    let mut rels = std::collections::HashMap::new();
    rels.insert("emp<->departmentBasic".to_string(), "INNER JOIN @table ON @1.department_basic_id = @2.id".to_string());

    let root = parser::parse_json(json_input, None).expect("parse");
    let sql_gen = generator::SqlGenerator::new(Some(wl), Some(rels));
    let result = sql_gen.generate(root).expect("generate");

    let sql = result.sql.as_ref().unwrap();
    println!("Custom key SQL:\n{}", sql);

    // {nn:name} → json_build_object('nn', name)
    assert!(sql.contains("json_build_object('nn', name)"), "Custom key 'nn' for column 'name'");
    // {title:name, key:id} → json_build_object('title', name, 'key', id)
    assert!(sql.contains("json_build_object('title', name, 'key', id)"), "Multi custom key mapping");
    // [name] → json_build_object('name', name)
    assert!(sql.contains("json_build_object('name', name)"), "[name] syntax still works");
    // string_agg for bare column
    assert!(sql.contains("string_agg(name::text"), "Bare column → string_agg");
}

#[test]
fn test_security_fixes() {
    let mut wl = indexmap::IndexMap::new();
    wl.insert("employee:emp".to_string(), json!(["id", "status", "name", "role"]));
    let mut rels = std::collections::HashMap::new();

    // ── FIX #1: CASE SELECT without space ──────────────────────────────────
    let json1 = r#"{"@data[]":{"@source":"emp","@fields":{"x":"CASE WHEN(SELECT(1))=1 THEN id ELSE id END"}}}"#;
    let root1 = parser::parse_json(json1, None).unwrap();
    let r1 = generator::SqlGenerator::new(Some(wl.clone()), Some(rels.clone())).generate(root1);
    assert!(r1.is_err(), "CASE SELECT( should be blocked: {:?}", r1);

    // ── FIX #2: IN operator comma split ────────────────────────────────────
    let src2 = parser::parse_source("emp[role: in (1, 2, 3), status: 1]");
    assert_eq!(src2.filters.len(), 2, "IN filter must be parsed as one unit, not split");
    assert_eq!(src2.filters[0].operator, "in");
    assert_eq!(src2.filters[0].value.trim(), "(1, 2, 3)");

    // ── FIX #3: $order must be validated at parse time ─────────────────────
    let src3a = parser::parse_source("emp[$order: id DESC]");
    assert!(src3a.order.is_some(), "$order valid");
    let src3b = parser::parse_source("emp[$order: id; DROP TABLE users]");
    assert!(src3b.order.is_none(), "malicious $order must be discarded");

    // ── FIX #4: $join must only accept known values ────────────────────────
    let src4a = parser::parse_source("emp[$join: left]");
    assert_eq!(src4a.join_type.as_deref(), Some("left"));
    let src4b = parser::parse_source("emp[$join: LEFT UNION SELECT 1]");
    assert!(src4b.join_type.is_none(), "malicious $join must be discarded");

    // ── FIX #5: SELECT in any field blocked by threats ─────────────────────
    let json5 = r#"{"@data":{"@source":"emp","@fields":{"x":"SELECT id FROM emp"}}}"#;
    let root5 = parser::parse_json(json5, None).unwrap();
    let r5 = generator::SqlGenerator::new(Some(wl.clone()), Some(rels.clone())).generate(root5);
    assert!(r5.is_err(), "SELECT in @fields must be blocked");

    // ── FIX #9: $limit capped at MAX_QUERY_LIMIT (10_000) ─────────────────
    let src9 = parser::parse_source("emp[$limit: 18446744073709551615]");
    assert!(src9.limit.unwrap() <= 10_000, "$limit must be capped");

    println!("All security fix tests passed ✓");
}

#[test]
fn test_security_fix_10_local_alias() {
    // Setup: flatten child puts "title" → "pos.name_latin" into local_aliases.
    // The parent expression CONCAT(title, ' ', id) must:
    //   - allow "title" (local alias, its value "pos.name_latin" is safe)
    //   - still validate "id" against the whitelist
    let json_input = r#"{
        "@data[]": {
            "@source": "emp[status: 1, $limit: 2]",
            "@fields": {
                "id": "id",
                "label": "CONCAT(title, ' ', id)"
            },
            "pos_info": {
                "@source": "pos",
                "@flatten": true,
                "@fields": { "title": "name_latin" }
            }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert("employee:emp".to_string(), json!(["id", "status"]));
    wl.insert("position:pos".to_string(), json!(["name_latin"]));

    let mut rels = std::collections::HashMap::new();
    rels.insert("emp<->pos".to_string(), "LEFT JOIN @table ON @1.pos_id = @2.id".to_string());

    let root = parser::parse_json(json_input, None).unwrap();
    let sql_gen = generator::SqlGenerator::new(Some(wl), Some(rels));
    let result = sql_gen.generate(root).expect("should succeed — title is a valid local alias");

    let sql = result.sql.as_ref().unwrap();
    println!("Fix #10 SQL:\n{}", sql);

    // "title" must be substituted with "pos.name_latin" (alias value), not left as-is
    assert!(sql.contains("pos.name_latin"), "alias must be substituted");
    assert!(!sql.contains("emp.title"), "raw alias key must not appear as emp column");
}

