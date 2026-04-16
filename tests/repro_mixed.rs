use indexmap::IndexMap;
use json_to_sql::parser;
use json_to_sql::generator::SqlGenerator;
use serde_json::json;

#[test]
fn test_mixed_macro_and_explicit_children() {
    let macros = json!({
        "my_macro": {
            "@source": "table_macro",
            "@fields": ["*"],
            "macro_child": {
                "@source": "table_sub",
                "@fields": ["id"]
            }
        }
    });

    // Request with explicit children AND macro call with custom fields
    let json_input = json!({
        "@data": {
            "@source": "table_root",
            "@fields": {"id": "id"},
            "explicit_child": {
                "@source": "table_explicit",
                "@fields": ["name"]
            },
            "macro_node": {
                "@source": "my_macro",
                "@fields": {"info": "id"}
            }
        }
    });

    let mut macro_map = IndexMap::new();
    for (k, v) in macros.as_object().unwrap() { macro_map.insert(k.clone(), v.clone()); }
    
    let mut wl = IndexMap::new();
    wl.insert("table_root".to_string(), json!(["*"]));
    wl.insert("table_macro".to_string(), json!(["*"]));
    wl.insert("table_sub".to_string(), json!(["*"]));
    wl.insert("table_explicit".to_string(), json!(["*"]));

    let mut rels = std::collections::HashMap::new();
    rels.insert("table_root->table_explicit".to_string(), "LEFT JOIN @table ON @1.id = @2.root_id".to_string());
    rels.insert("table_root->table_macro".to_string(), "LEFT JOIN @table ON @1.id = @2.root_id".to_string());
    rels.insert("table_macro->table_sub".to_string(), "LEFT JOIN @table ON @1.id = @2.macro_id".to_string());

    let root = parser::parse_json(&json_input.to_string(), Some(&macro_map)).unwrap();
    let mut generator = SqlGenerator::new(Some(wl), Some(rels));
    let sql = generator.generate(root).unwrap().sql.unwrap();
    
    println!("SQL:\n{}", sql);

    // 1. Root level: should have 'id' AND 'explicit_child' AND 'macro_node'.
    // Even though @data has @fields, 'explicit_child' should be included because it's not from a macro.
    assert!(sql.contains("'id', table_root.id"));
    assert!(sql.contains("'explicit_child', json_build_object('name', table_explicit.name)"));
    assert!(sql.contains("'macro_node', json_build_object('info', table_macro.id)"));

    // 2. Macro level (macro_node): user provided @fields mapping but did NOT mention 'macro_child'.
    // Since 'macro_child' is from the macro, it SHOULD be excluded.
}
