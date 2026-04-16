use json_to_sql::parser;
use json_to_sql::generator::SqlGenerator;

#[test]
fn test_manual() {
    let json_input = r#"{
    "@data": {
        "@source": "emp[status: 1, id: 1000..2145, $limit: 20, $order: id DESC]",
        "@fields": {
            "id": "id",
            "full_name": "full_name",
            "passport": "jshshir",
            "tug‘ilgan sana": "birthDay"
        },
        "position": {
            "@source": "departmentStaffPosition[is_current: true]",
            "@fields": {
                "id": "id",
                "begin_date": "TO_CHAR(TO_TIMESTAMP(start_time), 'DD.MM.YYYY')"
            },
            "test": {
                "@source": "org[status: 1]",
                "@flatten": true,
                "@fields": {
                    "viloyat boshqarma": "name"
                }
            },
            "1": {
                "@source": "innerOrg[status: 1]",
                "@flatten": true,
                "@fields": {
                    "tuman boshqarma": "name_uz"
                }
            },
            "2": {
                "@source": "departmentBasic[status: 1]",
                "@flatten": true,
                "@fields": {
                    "bo‘lim": "name_uz"
                }
            },
            "3": {
                "@source": "staffPositionBasic[status: 1]",
                "@flatten": true,
                "0": {
                    "@source": "staffPosition[status: 1]",
                    "@flatten": true,
                    "@fields": {
                        "name": "name_uz"
                    }
                }
            }
        },
        "educations[]": {
            "@source": "education[$limit: 15, $order: id DESC]",
            "@fields": {
                "id": "unique",
                "diploma_type_name": "diploma_type"
            }
        }
    }
}"#;
    
    let root = parser::parse_json(json_input, None).expect("Should parse");
    
    let mut wl = indexmap::IndexMap::new();
    wl.insert("emp".to_string(), serde_json::json!(["*"]));
    wl.insert("departmentStaffPosition".to_string(), serde_json::json!(["*"]));
    wl.insert("org".to_string(), serde_json::json!(["*"]));
    wl.insert("innerOrg".to_string(), serde_json::json!(["*"]));
    wl.insert("departmentBasic".to_string(), serde_json::json!(["*"]));
    wl.insert("staffPositionBasic".to_string(), serde_json::json!(["*"]));
    wl.insert("staffPosition".to_string(), serde_json::json!(["*"]));
    wl.insert("education".to_string(), serde_json::json!(["*"]));
    
    let mut rels = std::collections::HashMap::new();
    rels.insert("emp->education".to_string(), "LEFT JOIN @table ON @1.id=@2.emp_id".to_string());
    rels.insert("emp->departmentStaffPosition".to_string(), "LEFT JOIN @table ON @1.a=@2.b".to_string());
    rels.insert("departmentStaffPosition<->org".to_string(), "LEFT JOIN @table ON @1.c=@2.d".to_string());
    rels.insert("departmentStaffPosition<->innerOrg".to_string(), "LEFT JOIN @table ON @1.c=@2.d".to_string());
    rels.insert("departmentStaffPosition<->departmentBasic".to_string(), "LEFT JOIN @table ON @1.c=@2.d".to_string());
    rels.insert("departmentStaffPosition<->staffPositionBasic".to_string(), "LEFT JOIN @table ON @1.c=@2.d".to_string());
    rels.insert("staffPositionBasic<->staffPosition".to_string(), "LEFT JOIN @table ON @1.c=@2.d".to_string());
    
    let gen_inst = SqlGenerator::new(Some(wl), Some(rels));
    match gen_inst.generate(root) {
        Ok(result) => println!("Success:\n{}", result.sql.unwrap()),
        Err(e) => println!("Error:\n{}", e),
    }
}
