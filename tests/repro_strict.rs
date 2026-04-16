use indexmap::IndexMap;
use json_to_sql::parser;
use json_to_sql::generator::SqlGenerator;
use serde_json::json;

#[test]
fn test_macro_strict_selection() {
    let relations = json!({
        "emp-><-departmentStaffPosition": "@join @table ON @1.id = @2.employee_id",
        "departmentStaffPosition<->org": "@join @table ON @1.org_id = @2.id",
        "departmentStaffPosition<->innerOrg": "@join @table ON @1.inner_org_id = @2.id"
    });

    let macros = json!({
        "positionCteTable": {
            "@source": "departmentStaffPosition[is_current: true]",
            "@fields": ["*"],
            "test": {
                "@source": "org[status: 1]",
                "@fields": {
                    "viloyat_boshqarma": "name"
                }
            },
            "1": {
                "@source": "innerOrg[status: 1]",
                "@flatten": true,
                "@fields": {
                    "tuman_boshqarma": "name_uz"
                }
            }
        }
    });

    // Case 1: user provides @fields mapping and does NOT ask for 'test'.
    // Result should NOT contain 'test'.
    let json_input_1 = json!({
        "@data": {
            "@source": "emp[status: 1]",
            "@fields": ["*"],
            "position": {
                "@source": "positionCteTable[is_current: true, $join: <-]",
                "@fields": {"red": "test"}
            }
        }
    });

    // Case 2: user provides @fields mapping and DOES ask for 'test' by name.
    // Result should contain 'test' aliased as 'my_test_field'.
    let json_input_2 = json!({
        "@data": {
            "@source": "emp[status: 1]",
            "@fields": ["*"],
            "position": {
                "@source": "positionCteTable[is_current: true, $join: FULL]",
                "@fields": {
                    "output_id": "id",
                    "my_test_field": "test"
                }
            }
        }
    });

    let mut macro_map = IndexMap::new();
    for (k, v) in macros.as_object().unwrap() { macro_map.insert(k.clone(), v.clone()); }
    let mut rel_map = std::collections::HashMap::new();
    for (k, v) in relations.as_object().unwrap() { rel_map.insert(k.clone(), v.as_str().unwrap().to_string()); }
    let mut wl = IndexMap::new();
    wl.insert("emp".to_string(), json!(["*"]));
    wl.insert("departmentStaffPosition".to_string(), json!(["*"]));
    wl.insert("org".to_string(), json!(["*"]));
    wl.insert("innerOrg".to_string(), json!(["*"]));

    // Execution 1
    let root1 = parser::parse_json(&json_input_1.to_string(), Some(&macro_map)).unwrap();
    let gen1 = SqlGenerator::new(Some(wl.clone()), Some(rel_map.clone()));
    let res1 = gen1.generate(root1).unwrap().sql.unwrap();
    println!("SQL 1:\n{}", res1);
    
    // In SQL 1, inside 'position', it should have 'red' mapped to 'test' structure.
    assert!(res1.contains("'red', json_build_object('viloyat_boshqarma', org.name)"));
    assert!(!res1.contains("'test', json_build_object"));

    // Execution 2
    let root2 = parser::parse_json(&json_input_2.to_string(), Some(&macro_map)).unwrap();
    let gen2 = SqlGenerator::new(Some(wl), Some(rel_map));
    let res2 = gen2.generate(root2).unwrap().sql.unwrap();
    println!("SQL 2:\n{}", res2);
    
    // In SQL 2, it should have 'output_id' and 'my_test_field' mapped to 'json_build_object(...)'.
    assert!(res2.contains("'output_id', departmentStaffPosition.id"));
    assert!(res2.contains("'my_test_field', json_build_object('viloyat_boshqarma', org.name)"));
}
