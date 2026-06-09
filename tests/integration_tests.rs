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
    assert!(sql_str.contains("json_build_object("));
    // DISTINCT ON deduplicates root rows multiplied by regular JOINs (Bug 5 fix)
    assert!(sql_str.contains("DISTINCT ON (personal._uaq_rn)"));
    assert!(sql_str.contains("ROW_NUMBER() OVER () AS _uaq_rn"));
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
    let json_input = "{\"@info\": [\"@tables[*]\", \"@relations\"]}\0".as_ptr() as *const c_char;
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
    assert!(sql.contains("input_json AS"));
    assert!(sql.contains("CONCAT(name)"));
    
    // Relations are embedded as ::jsonb inside the generated SQL, not a separate field
    assert!(sql.contains("emp->org"));
    assert!(sql.contains("org->dept"));
    
    uaq_free_string(result_ptr);
}

#[test]
fn test_info_tables_empty_brackets_error() {
    // @tables (bare) and @tables[] (empty) must both return isOk=false
    let json_input = "{\"@info\": [\"@tables[]\"]}\0".as_ptr() as *const c_char;
    let whitelist_input = "{\"employee:emp\": [\"id\"]}\0".as_ptr() as *const c_char;

    let result_ptr = uaq_parse(json_input, whitelist_input, std::ptr::null(), std::ptr::null());
    assert!(!result_ptr.is_null());

    let result_str = unsafe { CStr::from_ptr(result_ptr).to_str().unwrap().to_string() };
    uaq_free_string(result_ptr);

    let v: serde_json::Value = serde_json::from_str(&result_str).unwrap();
    assert_eq!(v["isOk"], false, "Empty @tables[] must return error: {}", result_str);
    let msg = v["message"].as_str().unwrap_or("");
    assert!(!msg.is_empty(), "Error message must not be empty");
    println!("Empty brackets error: {}", msg);

    // @tables (bare, no brackets) must also return an error
    let json_bare = "{\"@info\": [\"@tables\"]}\0".as_ptr() as *const c_char;
    let wl = "{\"employee:emp\": [\"id\"]}\0".as_ptr() as *const c_char;
    let ptr2 = uaq_parse(json_bare, wl, std::ptr::null(), std::ptr::null());
    let s2 = unsafe { CStr::from_ptr(ptr2).to_str().unwrap().to_string() };
    uaq_free_string(ptr2);
    let v2: serde_json::Value = serde_json::from_str(&s2).unwrap();
    assert_eq!(v2["isOk"], false, "Bare @tables must also return error: {}", s2);
}

#[test]
fn test_info_tables_prefix_filter() {
    // @tables[employee~] should return all whitelist aliases that start with "employee"
    let whitelist_str = r#"{
        "manuals_employee_action_type:employeeActionType": {"id": "id", "name": "name_lt"},
        "manuals_employee_status:employeeStatus":          {"id": "id", "label": "label"},
        "personal_info:personal":                          ["id", "full_name"]
    }"#;

    let info_arr = serde_json::json!(["@tables[employee~]"]);
    let arr = info_arr.as_array().unwrap();
    let result = json_to_sql::info::process_info_request(arr, Some(whitelist_str), None, None);

    assert_eq!(result["isOk"], true, "prefix filter must succeed: {:?}", result);
    let sql = result["sql"].as_str().unwrap_or("");
    println!("Prefix filter SQL:\n{}", sql);

    // Both "employee" aliases must appear in the generated SQL
    assert!(sql.contains("employeeActionType"), "employeeActionType must be included");
    assert!(sql.contains("employeeStatus"),     "employeeStatus must be included");
    // "personal" must NOT appear (doesn't start with "employee")
    assert!(!sql.contains("personal"),          "personal must be excluded");
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

#[test]
fn test_lateral_list_multi_hop() {
    // department[] is a list child reachable only via an intermediate table
    // (emp → dept_pos → departmentBasic). build_lateral_subquery must discover
    // the path via BFS and build inner JOINs correctly instead of erroring.
    let json_input = r#"{
        "@data[]": {
            "@source": "emp[status: 1, $limit: 4]",
            "@fields": { "id": "id", "full_name": "full_name" },
            "department[]": {
                "@source": "departmentBasic",
                "@fields": {
                    "id":   "id",
                    "name": "name"
                }
            }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert("employee:emp".to_string(), json!(["id", "full_name", "status"]));
    wl.insert("employee_department_staff_position:dept_pos".to_string(), json!(["*"]));
    wl.insert("shtat_department_basic:departmentBasic".to_string(), json!(["id", "name"]));

    let mut rels = std::collections::HashMap::new();
    rels.insert("emp-><-dept_pos".to_string(), "INNER JOIN @table ON @1.id = @2.employee_id AND @2.status = 1".to_string());
    rels.insert("dept_pos<->departmentBasic".to_string(), "INNER JOIN @table ON @1.department_basic_id = @2.id".to_string());

    let root = parser::parse_json(json_input, None).unwrap();
    let sql_gen = generator::SqlGenerator::new(Some(wl), Some(rels));
    let result = sql_gen.generate(root).expect("list child via multi-hop should work");

    let sql = result.sql.as_ref().unwrap();
    println!("Multi-hop list SQL:\n{}", sql);

    // Intermediate table must appear inside the LATERAL subquery
    assert!(sql.contains("LEFT JOIN LATERAL"), "must use LATERAL for list child");
    assert!(sql.contains("employee_department_staff_position AS dept_pos"), "intermediate table must be joined");
    assert!(sql.contains("shtat_department_basic AS departmentBasic"), "leaf table must be FROM inside lateral");
    // Correlation ties back to outer emp
    assert!(sql.contains("emp.id"), "lateral must correlate to outer emp");
}


#[test]
fn test_operation_object_update_and_insert() {
    let json_input = r#"{
        "@operation": {
            "emp[id: 4]": {
                "l_name": "Soliyev",
                "f_name": "Ali"
            },
            "departmentBasic": {
                "dep_name": "IT"
            }
        }
    }"#;

    let whitelist = r#"{
        "employees:emp": {
            "l_name": "last_name",
            "f_name": "first_name",
            "id":     "emp_id"
        },
        "departments:departmentBasic": {
            "dep_name": "name"
        }
    }"#;

    let json_c = std::ffi::CString::new(json_input).unwrap();
    let wl_c   = std::ffi::CString::new(whitelist).unwrap();

    let ptr = uaq_parse(json_c.as_ptr(), wl_c.as_ptr(), std::ptr::null(), std::ptr::null());
    assert!(!ptr.is_null());

    let result_str = unsafe { CStr::from_ptr(ptr).to_str().unwrap().to_string() };
    uaq_free_string(ptr);

    let v: serde_json::Value = serde_json::from_str(&result_str).unwrap();
    assert_eq!(v["isOk"], true, "Expected isOk=true, got: {}", result_str);

    let updates = v["data"]["update"].as_array().unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["filter"]["emp_id"], 4);
    assert_eq!(updates[0]["employees"]["last_name"], "Soliyev");
    assert_eq!(updates[0]["employees"]["first_name"], "Ali");

    let inserts = v["data"]["insert"].as_array().unwrap();
    assert_eq!(inserts.len(), 1);
    assert_eq!(inserts[0]["departments"]["name"], "IT");

    assert_eq!(v["rejected"].as_array().unwrap().len(), 0);
}

#[test]
fn test_operation_array_same_table_twice() {
    let json_input = r#"{
        "@operation": [
            { "emp": { "l_name": "Ali",   "f_name": "Vali" } },
            { "emp": { "l_name": "Karim", "f_name": "To'lqin" } }
        ]
    }"#;

    let whitelist = r#"{
        "employees:emp": {
            "l_name": "last_name",
            "f_name": "first_name"
        }
    }"#;

    let json_c = std::ffi::CString::new(json_input).unwrap();
    let wl_c   = std::ffi::CString::new(whitelist).unwrap();

    let ptr = uaq_parse(json_c.as_ptr(), wl_c.as_ptr(), std::ptr::null(), std::ptr::null());
    let result_str = unsafe { CStr::from_ptr(ptr).to_str().unwrap().to_string() };
    uaq_free_string(ptr);

    let v: serde_json::Value = serde_json::from_str(&result_str).unwrap();
    assert_eq!(v["isOk"], true, "Expected isOk=true, got: {}", result_str);

    let inserts = v["data"]["insert"].as_array().unwrap();
    assert_eq!(inserts.len(), 2, "Both rows must be inserted");
    assert_eq!(inserts[0]["employees"]["last_name"], "Ali");
    assert_eq!(inserts[1]["employees"]["last_name"], "Karim");
}

#[test]
fn test_operation_rejected_unknown_field() {
    let json_input = r#"{
        "@operation": {
            "emp": { "l_name": "Ali", "unknown_col": "hacker" }
        }
    }"#;

    let whitelist = r#"{
        "employees:emp": { "l_name": "last_name" }
    }"#;

    let json_c = std::ffi::CString::new(json_input).unwrap();
    let wl_c   = std::ffi::CString::new(whitelist).unwrap();

    let ptr = uaq_parse(json_c.as_ptr(), wl_c.as_ptr(), std::ptr::null(), std::ptr::null());
    let result_str = unsafe { CStr::from_ptr(ptr).to_str().unwrap().to_string() };
    uaq_free_string(ptr);

    let v: serde_json::Value = serde_json::from_str(&result_str).unwrap();
    assert_eq!(v["isOk"], true);

    let rejected = v["rejected"].as_array().unwrap();
    assert!(rejected.contains(&serde_json::json!("unknown_col")));

    let inserts = v["data"]["insert"].as_array().unwrap();
    assert_eq!(inserts[0]["employees"]["last_name"], "Ali");
}

#[test]
fn test_operation_threat_in_value() {
    let json_input = r#"{
        "@operation": {
            "emp": { "l_name": "Ali'; DROP TABLE employees; --" }
        }
    }"#;

    let whitelist = r#"{ "employees:emp": { "l_name": "last_name" } }"#;

    let json_c = std::ffi::CString::new(json_input).unwrap();
    let wl_c   = std::ffi::CString::new(whitelist).unwrap();

    let ptr = uaq_parse(json_c.as_ptr(), wl_c.as_ptr(), std::ptr::null(), std::ptr::null());
    let result_str = unsafe { CStr::from_ptr(ptr).to_str().unwrap().to_string() };
    uaq_free_string(ptr);

    let v: serde_json::Value = serde_json::from_str(&result_str).unwrap();
    assert_eq!(v["isOk"], false, "Should fail on SQL injection attempt");
}

#[test]
fn test_inner_join_limit_in_subquery() {
    // Bug: when the root has $limit AND a child uses INNER JOIN, the engine previously
    // applied LIMIT to the root table BEFORE the INNER JOIN filter, causing fewer results
    // than requested (e.g. $limit:10 → only 4 results because 6 were filtered by INNER JOIN).
    //
    // Fix: INNER JOINs are included inside the root subquery so LIMIT applies AFTER
    // filtering. They are then re-joined in the outer query for column access.
    let json_input = r#"{
        "@data[]": {
            "@source": "roe[orgId: 2, $limit: 10]",
            "employee": {
                "@source": "employee",
                "@flatten": true,
                "@fields": {
                    "id": "id",
                    "lastName": "last_name"
                }
            },
            "dept": {
                "@source": "department",
                "@flatten": true,
                "@fields": { "deptName": "name" }
            }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert("relation_organization_employee:roe".to_string(),
        json!({"orgId": "organization_id", "employee_id": "employee_id", "status": "status"}));
    wl.insert("employee".to_string(), json!(["id", "last_name"]));
    wl.insert("department".to_string(), json!(["id", "name", "employee_id"]));

    let mut rels = std::collections::HashMap::new();
    // INNER JOIN: roe -><- employee (filtering join — must be inside subquery)
    rels.insert(
        "roe-><-employee".to_string(),
        "@join @table ON @2.id = @1.employee_id".to_string(),
    );
    // LEFT JOIN: roe -> department (enriching join — stays in outer query)
    rels.insert(
        "roe->department".to_string(),
        "@join @table ON @2.employee_id = @1.employee_id".to_string(),
    );

    let root = parser::parse_json(json_input, None).unwrap();
    let result = generator::SqlGenerator::new(Some(wl), Some(rels))
        .generate(root)
        .expect("should generate SQL");

    let sql = result.sql.as_ref().unwrap();
    println!("INNER JOIN limit SQL:\n{}", sql);

    // INNER JOIN must appear INSIDE the root subquery so LIMIT applies after filtering
    let subquery_start = sql.find("ROW_NUMBER() OVER () AS _uaq_rn").expect("subquery must exist");
    let limit_pos = sql.find("LIMIT 10").expect("LIMIT must be present");
    let inner_join_pos = sql.find("INNER JOIN").expect("INNER JOIN must be present");
    assert!(
        inner_join_pos < limit_pos,
        "INNER JOIN must appear before LIMIT (inside subquery)"
    );
    assert!(
        inner_join_pos > subquery_start - 200,
        "INNER JOIN must be close to ROW_NUMBER (inside subquery body)"
    );

    // LEFT JOIN (department) must stay in the outer query — after the subquery closing paren
    let subquery_end = sql.rfind(") AS roe").expect("subquery must close with alias");
    let left_join_pos = sql.find("LEFT JOIN department").expect("LEFT JOIN must exist");
    assert!(
        left_join_pos > subquery_end,
        "LEFT JOIN must be in the outer query, not inside the subquery"
    );

    // DISTINCT ON must be present (because of the LEFT JOIN that can multiply rows)
    assert!(sql.contains("DISTINCT ON (roe._uaq_rn)"), "DISTINCT ON required for LEFT JOINs");

    // employee columns must be accessible in the outer SELECT
    assert!(sql.contains("employee.last_name"), "employee columns must be reachable in outer query");

    // params: orgId filter
    assert!(result.params.as_ref().unwrap().contains_key("p1"), "filter param must exist");
}

#[test]
fn test_whitelist_default_filter() {
    // "manuals_employee_action_type:employeeActionType[status:1]"
    // The "[status:1]" part means: always inject WHERE employeeActionType.status = :p1
    // "status" does NOT need to be in the allowed columns list.
    let json_input = r#"{
        "@data[]": {
            "@source": "employeeActionType",
            "@fields": { "id": "id", "name": "name" }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert(
        "manuals_employee_action_type:employeeActionType[status:1]".to_string(),
        json!({"id": "id", "name": "name_lt"}),
    );

    let root = parser::parse_json(json_input, None).unwrap();
    let result = generator::SqlGenerator::new(Some(wl.clone()), None)
        .generate(root)
        .expect("should generate SQL");

    let sql = result.sql.as_ref().unwrap();
    let params = result.params.as_ref().unwrap();
    println!("Default filter SQL:\n{}", sql);

    // Default filter must appear as WHERE condition
    assert!(sql.contains("employeeActionType.status = :p"), "default filter must be in WHERE");
    // The param value must be "1" (the backend-defined value)
    let status_param = params.values().find(|v| v.as_str() == Some("1"));
    assert!(status_param.is_some(), "filter value '1' must be in params");

    // Now test with list child (lateral subquery path)
    let json_lateral = r#"{
        "@data[]": {
            "@source": "emp[$limit: 5]",
            "@fields": { "id": "id" },
            "actionTypes[]": {
                "@source": "employeeActionType",
                "@fields": { "id": "id", "name": "name" }
            }
        }
    }"#;

    let mut wl2 = indexmap::IndexMap::new();
    wl2.insert("employee:emp".to_string(), json!(["id"]));
    wl2.insert(
        "manuals_employee_action_type:employeeActionType[status:1]".to_string(),
        json!({"id": "id", "name": "name_lt"}),
    );

    let mut rels = std::collections::HashMap::new();
    rels.insert(
        "emp->employeeActionType".to_string(),
        "LEFT JOIN @table ON @1.id = @2.employee_id".to_string(),
    );

    let root2 = parser::parse_json(json_lateral, None).unwrap();
    let result2 = generator::SqlGenerator::new(Some(wl2), Some(rels))
        .generate(root2)
        .expect("lateral with default filter should work");

    let sql2 = result2.sql.as_ref().unwrap();
    println!("Lateral default filter SQL:\n{}", sql2);
    assert!(sql2.contains("LEFT JOIN LATERAL"), "must use LATERAL for list child");
    assert!(sql2.contains("employeeActionType.status = :p"), "default filter must be inside lateral WHERE");
}

#[test]
fn test_whitelist_default_order() {
    // Whitelist key with $order: default ORDER BY applied when user doesn't supply one
    let json_no_order = r#"{
        "@data[]": {
            "@source": "educationInstitutionType",
            "@fields": { "id": "id", "nameUz": "nameUz" }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert(
        "manuals_educational_institution_type:educationInstitutionType[status: 1, $order: order_number ASC]".to_string(),
        json!({"id": "id", "nameUz": "name_uz"}),
    );

    let root = parser::parse_json(json_no_order, None).unwrap();
    let result = generator::SqlGenerator::new(Some(wl.clone()), None)
        .generate(root)
        .expect("should generate SQL");

    let sql = result.sql.as_ref().unwrap();
    println!("Default order SQL:\n{}", sql);
    assert!(sql.contains("ORDER BY"), "default order must appear");
    assert!(sql.contains("order_number ASC"), "must use whitelist default order value");

    // When user supplies $order in @source, it must override the whitelist default
    let json_with_order = r#"{
        "@data[]": {
            "@source": "educationInstitutionType[$order: id DESC]",
            "@fields": { "id": "id", "nameUz": "nameUz" }
        }
    }"#;

    let root2 = parser::parse_json(json_with_order, None).unwrap();
    let result2 = generator::SqlGenerator::new(Some(wl), None)
        .generate(root2)
        .expect("should generate SQL with user order");

    let sql2 = result2.sql.as_ref().unwrap();
    println!("User-override order SQL:\n{}", sql2);
    assert!(sql2.contains("ORDER BY"), "order must appear");
    assert!(sql2.contains("id DESC"), "user-supplied order must win");
    assert!(!sql2.contains("order_number"), "whitelist default must NOT appear when user supplies order");
}

#[test]
fn test_whitelist_default_filter_null_operators() {
    // Whitelist key: "table:alias[status: 1, parent_id: null]"
    // parent_id: null  → IS NULL (no param)
    // parent_id: !null → IS NOT NULL (no param)
    let json_input = r#"{
        "@data[]": {
            "@source": "educationAcademicDegreeCategory",
            "@fields": { "id": "id", "parentId": "parentId" }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert(
        "manuals_academic_degree:educationAcademicDegreeCategory[status: 1, parent_id: null]".to_string(),
        json!({"id": "id", "parentId": "parent_id"}),
    );

    let root = parser::parse_json(json_input, None).unwrap();
    let result = generator::SqlGenerator::new(Some(wl), None)
        .generate(root)
        .expect("should generate SQL");

    let sql = result.sql.as_ref().unwrap();
    let params = result.params.as_ref().unwrap();
    println!("Whitelist null default filter SQL:\n{}", sql);

    assert!(sql.contains("IS NULL"), "parent_id: null must generate IS NULL");
    assert!(!sql.contains("= 'null'"), "must not emit string = 'null'");
    // Only one param: status = :p1 (IS NULL generates no param)
    assert_eq!(params.len(), 1, "only status param should exist");

    // !null → IS NOT NULL
    let mut wl2 = indexmap::IndexMap::new();
    wl2.insert(
        "manuals_academic_degree:educationAcademicDegree[status: 1, parent_id: !null]".to_string(),
        json!({"id": "id", "parentId": "parent_id"}),
    );

    let json_input2 = r#"{
        "@data[]": {
            "@source": "educationAcademicDegree",
            "@fields": { "id": "id", "parentId": "parentId" }
        }
    }"#;

    let root2 = parser::parse_json(json_input2, None).unwrap();
    let result2 = generator::SqlGenerator::new(Some(wl2), None)
        .generate(root2)
        .expect("!null should generate IS NOT NULL");

    let sql2 = result2.sql.as_ref().unwrap();
    println!("Whitelist !null default filter SQL:\n{}", sql2);
    assert!(sql2.contains("IS NOT NULL"), "parent_id: !null must generate IS NOT NULL");
    assert!(!sql2.contains("= '!null'"), "must not emit string = '!null'");
}

#[test]
fn test_total_count_sql() {
    // Simple list: total must be SELECT COUNT(*) FROM table WHERE <root filters>
    let json_input = r#"{
        "@data[]": {
            "@source": "emp[status: 1, $limit: 10]",
            "@fields": { "id": "id", "name": "full_name" }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert("employee:emp".to_string(), json!(["id", "full_name", "status"]));

    let root = parser::parse_json(json_input, None).unwrap();
    let result = generator::SqlGenerator::new(Some(wl), None)
        .generate(root)
        .expect("should generate SQL");

    let total = result.total.as_ref().expect("total must be present for list query");
    let params = result.params.as_ref().unwrap();
    println!("Total SQL: {}", total);

    assert!(total.starts_with("SELECT COUNT(*)"), "total must be a COUNT query");
    assert!(total.contains("employee AS emp"),    "total must reference root table");
    assert!(total.contains("emp.status = :p"),    "total must include root filters");
    // $limit must NOT appear in total
    assert!(!total.contains("LIMIT"),  "total must not include LIMIT");
    assert!(!total.contains("OFFSET"), "total must not include OFFSET");
    // The filter param must exist
    assert!(params.values().any(|v| v.as_str() == Some("1")), "filter param must be in params");

    // total_params must only contain params referenced in total SQL
    let total_params = result.total_params.as_ref().expect("total_params must be present");
    println!("Total params: {:?}", total_params);
    for key in total_params.keys() {
        assert!(total.contains(&format!(":{}", key)), "total_params key '{}' must appear in total SQL", key);
    }

    // Test with a child that adds extra params (e.g. default filter on joined table).
    // total_params must NOT include child params — only root-level ones.
    let json_with_child = r#"{
        "@data[]": {
            "@source": "emp[status: 1, $limit: 5]",
            "@fields": { "id": "id" },
            "org": {
                "@source": "org[type: 2]",
                "@fields": { "name": "name" }
            }
        }
    }"#;
    let mut wl_child = indexmap::IndexMap::new();
    wl_child.insert("employee:emp".to_string(), json!(["id", "status"]));
    wl_child.insert("organization:org".to_string(), json!(["name", "type"]));
    let mut rels_child = std::collections::HashMap::new();
    rels_child.insert("emp->org".to_string(), "LEFT JOIN @table ON @1.org_id = @2.id".to_string());
    let root_child = parser::parse_json(json_with_child, None).unwrap();
    let result_child = generator::SqlGenerator::new(Some(wl_child), Some(rels_child))
        .generate(root_child).expect("should work");
    let total_child = result_child.total.as_ref().unwrap();
    let total_params_child = result_child.total_params.as_ref().unwrap();
    let all_params_child = result_child.params.as_ref().unwrap();
    println!("Child test total SQL: {}", total_child);
    println!("Child test total_params: {:?}", total_params_child);
    println!("Child test all params: {:?}", all_params_child);
    // main params has p1 (status:1) and p2 (type:2); total only references p1
    assert!(all_params_child.len() >= 2, "main params must include child filter param");
    for key in total_params_child.keys() {
        assert!(total_child.contains(&format!(":{}", key)),
            "total_params key '{}' must appear in total SQL", key);
    }
    // Child's filter param must NOT be in total_params
    for key in all_params_child.keys() {
        if !total_child.contains(&format!(":{}", key)) {
            assert!(!total_params_child.contains_key(key),
                "param '{}' is not in total SQL but appears in total_params", key);
        }
    }

    // INNER JOIN child filter MUST appear in total (it reduces root count)
    let json_inner = r#"{
        "@data[]": {
            "@source": "emp[status: 1, $limit: 5]",
            "@fields": { "id": "id" },
            "org": {
                "@source": "org[type: 2, $join: inner]",
                "@fields": { "name": "name" }
            }
        }
    }"#;
    let mut wl_inner = indexmap::IndexMap::new();
    wl_inner.insert("employee:emp".to_string(), json!(["id", "status"]));
    wl_inner.insert("organization:org".to_string(), json!(["name", "type"]));
    let mut rels_inner = std::collections::HashMap::new();
    rels_inner.insert("emp->org".to_string(), "LEFT JOIN @table ON @1.org_id = @2.id".to_string());
    let root_inner = parser::parse_json(json_inner, None).unwrap();
    let result_inner = generator::SqlGenerator::new(Some(wl_inner), Some(rels_inner))
        .generate(root_inner).expect("inner join total");
    let total_inner = result_inner.total.as_ref().unwrap();
    let tparams_inner = result_inner.total_params.as_ref().unwrap();
    let all_inner = result_inner.params.as_ref().unwrap();
    println!("INNER JOIN total SQL: {}", total_inner);
    println!("INNER JOIN totalParams: {:?}", tparams_inner);
    println!("INNER JOIN all params: {:?}", all_inner);
    // total must contain INNER JOIN and org condition
    assert!(total_inner.contains("INNER JOIN"), "INNER JOIN must be in total");
    assert!(total_inner.contains("org.type = :p"), "INNER JOIN child filter must be in total");
    // totalParams must include BOTH p1 (root) and p2 (inner child)
    assert!(tparams_inner.len() == 2, "totalParams must have both root and inner-child params, got: {:?}", tparams_inner);

    // Single object query (@data, not @data[]) must NOT have total
    let json_single = r#"{"@data": {"@source": "emp", "@fields": {"id": "id"}}}"#;
    let root2 = parser::parse_json(json_single, None).unwrap();
    let mut wl2 = indexmap::IndexMap::new();
    wl2.insert("employee:emp".to_string(), json!(["id"]));
    let result2 = generator::SqlGenerator::new(Some(wl2), None)
        .generate(root2)
        .expect("should generate SQL");
    assert!(result2.total.is_none(), "non-list query must not have total");
    assert!(result2.total_params.is_none(), "non-list query must not have total_params");
}

#[test]
fn test_or_group() {
    let wl = indexmap::IndexMap::from([
        ("employee".to_string(), json!(["id", "nameUz", "nameRu", "status"])),
    ]);

    // 1. Parse: $or: [...] — or_groups to'g'ri parse qilinishi
    let src = parser::parse_source("employee[$or: [nameUz: ~ Ali%, nameRu: ~ Ali%]]");
    assert_eq!(src.filters.len(), 0, "filters bo'sh bo'lishi kerak");
    assert_eq!(src.or_groups.len(), 1, "bitta or_group bo'lishi kerak");
    assert_eq!(src.or_groups[0].len(), 2, "or_group ichida 2 ta filter");
    assert_eq!(src.or_groups[0][0].field, "nameUz");
    assert_eq!(src.or_groups[0][0].operator, "like");
    assert_eq!(src.or_groups[0][1].field, "nameRu");
    assert_eq!(src.or_groups[0][1].operator, "like");

    // 2. SQL generatsiya: (nameUz LIKE :p1 OR nameRu LIKE :p2)
    let json_input = r#"{
        "@data[]": {
            "@source": "employee[$or: [nameUz: ~ Ali%, nameRu: ~ Ali%]]",
            "@fields": {"id": "id", "nameUz": "nameUz"}
        }
    }"#;
    let root = parser::parse_json(json_input, None).expect("parse bo'lishi kerak");
    let result = generator::SqlGenerator::new(Some(wl.clone()), None)
        .generate(root)
        .expect("SQL generatsiya bo'lishi kerak");
    let sql = result.sql.unwrap();
    println!("OR SQL: {}", sql);
    assert!(sql.contains("OR"), "SQL da OR bo'lishi kerak");
    assert!(sql.contains("LIKE"), "SQL da LIKE bo'lishi kerak");

    // 3. AND + OR birga
    let src2 = parser::parse_source("employee[status: 1, $or: [nameUz: ~ Vali%, nameRu: ~ Vali%]]");
    assert_eq!(src2.filters.len(), 1, "bitta AND filter");
    assert_eq!(src2.or_groups.len(), 1, "bitta OR group");

    let json2 = r#"{
        "@data[]": {
            "@source": "employee[status: 1, $or: [nameUz: ~ Vali%, nameRu: ~ Vali%]]",
            "@fields": {"id": "id"}
        }
    }"#;
    let root2 = parser::parse_json(json2, None).expect("parse bo'lishi kerak");
    let result2 = generator::SqlGenerator::new(Some(wl.clone()), None)
        .generate(root2)
        .expect("SQL generatsiya bo'lishi kerak");
    let sql2 = result2.sql.unwrap();
    println!("AND+OR SQL: {}", sql2);
    assert!(sql2.contains("OR"), "OR bo'lishi kerak");
    assert!(sql2.contains(":p1"), "AND filter parametri bo'lishi kerak");

    println!("OR group testlari o'tdi ✓");
}

#[test]
fn test_null_operators() {
    // field: null  → IS NULL
    // field: !null → IS NOT NULL
    let json_input = r#"{
        "@data[]": {
            "@source": "emp[deleted_at: null, manager_id: !null, status: 1]",
            "@fields": { "id": "id", "name": "last_name" }
        }
    }"#;

    let mut wl = indexmap::IndexMap::new();
    wl.insert(
        "employee:emp".to_string(),
        json!(["id", "last_name", "status", "deleted_at", "manager_id"]),
    );

    let root   = parser::parse_json(json_input, None).expect("parse failed");
    let result = generator::SqlGenerator::new(Some(wl), None)
        .generate(root)
        .expect("generate failed");

    let sql = result.sql.as_ref().unwrap();
    println!("NULL operators SQL:\n{}", sql);

    assert!(sql.contains("emp.deleted_at IS NULL"),     "IS NULL bo'lishi kerak");
    assert!(sql.contains("emp.manager_id IS NOT NULL"), "IS NOT NULL bo'lishi kerak");
    assert!(sql.contains(":p1"),                        "status uchun param bo'lishi kerak");

    // IS NULL / IS NOT NULL uchun qo'shimcha param yaratilmasligi kerak
    let params = result.params.as_ref().unwrap();
    assert_eq!(params.len(), 1, "faqat status uchun 1 ta param bo'lishi kerak");

    // Case-insensitive: NULL va !NULL ham ishlashi kerak
    let json_upper = r#"{
        "@data[]": {
            "@source": "emp[deleted_at: NULL, manager_id: !NULL]",
            "@fields": { "id": "id" }
        }
    }"#;
    let mut wl2 = indexmap::IndexMap::new();
    wl2.insert("employee:emp".to_string(), json!(["id", "deleted_at", "manager_id"]));

    let root2   = parser::parse_json(json_upper, None).expect("parse failed (upper)");
    let result2 = generator::SqlGenerator::new(Some(wl2), None)
        .generate(root2)
        .expect("generate failed (upper)");

    let sql2 = result2.sql.as_ref().unwrap();
    assert!(sql2.contains("IS NULL"),     "uppercase NULL: IS NULL bo'lishi kerak");
    assert!(sql2.contains("IS NOT NULL"), "uppercase !NULL: IS NOT NULL bo'lishi kerak");
    assert!(result2.params.as_ref().unwrap().is_empty(), "NULL uchun param bo'lmasligi kerak");

    println!("NULL operators testlari o'tdi ✓");
}

#[test]
fn test_array_fields_with_macro() {
    use json_to_sql::parser;
    use json_to_sql::generator;
    use indexmap::IndexMap;
    use serde_json::{json, Value};
    use std::collections::HashMap;

    let json_input = r#"{"@data":{"@source":"employeeDataCollector[id: 37566]","@fields":["jshshir"]}}"#;
    
    let whitelist_json: Value = json!({
        "employee[status: 1]": {"id":"id","jshshir":"jshshir","lastNameUz":"last_name"},
        "employee_department_military_degree:employeeCurrentDegree[status: 1]": {"id":"id","employeeId":"employee_id","degreeGivenTime":"degree_given_time"}
    });
    let whitelist: IndexMap<String, Value> = serde_json::from_value(whitelist_json).unwrap();
    
    let macros_json: Value = json!({
        "employeeDataCollector": {
            "@source": "employee",
            "@fields": {"id":"id","jshshir":"jshshir","lastNameUz":"last_name"},
            "0": {"@source":"employeeCurrentDegree","@fields":{"militaryDegreeDate":"degreeGivenTime"}}
        }
    });
    let macros: IndexMap<String, Value> = serde_json::from_value(macros_json).unwrap();
    
    let mut rels = HashMap::new();
    rels.insert("employee->employeeCurrentDegree".to_string(), "LEFT JOIN @table ON @1.id = @2.employee_id AND @2.status = 1".to_string());
    
    let root = parser::parse_json(json_input, Some(&macros)).expect("Should parse");
    let generator_inst = generator::SqlGenerator::new(Some(whitelist), Some(rels));
    let result = generator_inst.generate(root).expect("Should generate SQL");
    
    let sql = result.sql.as_ref().unwrap();
    println!("Generated SQL:\n{}", sql);
    println!("Params: {:?}", result.params);
    
    // Should only have jshshir in SELECT, not child macro fields
    assert!(sql.contains("'jshshir'"), "Should select jshshir field");
    // Macro children must NOT cause unnecessary JOINs in strict mode
    assert!(!sql.contains("employeeCurrentDegree"), "Macro child JOIN must be skipped in strict mode");
    assert!(!sql.contains("employee_department_military_degree"), "Macro child real table must not appear");
    // No spurious WHERE conditions from LEFT JOIN tables
    assert!(!sql.contains(":p3"), "No extra params from skipped macro children");
}
