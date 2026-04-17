use json_to_sql::api::uaq_parse;

fn main() {
    let input1 = r#"
    {
      "@data[]": {
        "@source": "emp[status: 1, id: 100..200, $limit: 10]",
        "@fields": { "id": "id", "jshshir": "jshshir", "full_name": "full_name" },
        
        "positions": {
          "@source": "departmentStaffPosition[status: 1, is_current: true]",
          "@fields": {
            "id": "id",
            "begin_date": "start_time"
          },
          "ishJoyi": {
            "@source": "positionCteTable",
            "@fields": ["*"]
          }
        },
        
        "educations[]": {
           "@source": "education[$limit: 10, $order: id DESC]",
           "@fields": {
              "id": "id",
              "diploma_type": "diploma_type_name"
           }
        }
      }
    }
    "#;

    let input2 = r#"
    {
      "@data[]": {
        "@source": "emp[status: 1, id: 100..200, $limit: 10]",
        "@fields": { "id": "id", "jshshir": "jshshir", "full_name": "full_name" },
        
        "positions": {
          "@source": "departmentStaffPosition",
          "@fields": {
            "id": "id",
            "begin_date": "start_time"
          },
          "ishJoyi": {
            "@source": "positionCteTable",
            "@fields": ["*"]
          }
        },
        
        "educations[]": {
           "@source": "education[$limit: 10, $order: id DESC]",
           "@fields": {
              "id": "id",
              "diploma_type": "diploma_type_name"
           }
        }
      }
    }
    "#;

    let whitelist = r#"
    {
        "emp": ["*"],
        "departmentStaffPosition": ["*"],
        "org": ["*"],
        "innerOrg": ["*"],
        "departmentBasic": ["*"],
        "education": ["*"]
    }
    "#;

    let relations = r#"
    {
        "emp->departmentStaffPosition": "LEFT JOIN @table ON @1.id = @2.emp_id",
        "departmentStaffPosition->org": "LEFT JOIN @table ON @1.org_id = @2.id",
        "departmentStaffPosition->innerOrg": "LEFT JOIN @table ON @1.inner_org_id = @2.id",
        "departmentStaffPosition->departmentBasic": "LEFT JOIN @table ON @1.dep_id = @2.id",
        "emp->education": "LEFT JOIN @table ON @1.id = @2.emp_id"
    }
    "#;

    let macros = r#"
    {
        "positionCteTable": {
            "@source": "departmentStaffPosition[status: 1, is_current: true]",
            "@fields": ["*"],
            "test": {
            "@source": "org[status: 1]",
            "@flatten": true,
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
            },
            "bo‘lim": {
            "@source": "departmentBasic[status: 1]",
            "@fields": {
                "name": "name",
                "has_children": "has_children"
            }
            }
        }
    }
    "#;

    use std::ffi::{CStr, CString};
    
    let c_input1 = CString::new(input1).unwrap();
    let c_input2 = CString::new(input2).unwrap();
    let c_ws = CString::new(whitelist).unwrap();
    let c_rel = CString::new(relations).unwrap();
    let c_mac = CString::new(macros).unwrap();

    let res1_ptr = uaq_parse(c_input1.as_ptr(), c_ws.as_ptr(), c_rel.as_ptr(), c_mac.as_ptr());
    let res2_ptr = uaq_parse(c_input2.as_ptr(), c_ws.as_ptr(), c_rel.as_ptr(), c_mac.as_ptr());

    let res1 = unsafe { CStr::from_ptr(res1_ptr).to_string_lossy().into_owned() };
    let res2 = unsafe { CStr::from_ptr(res2_ptr).to_string_lossy().into_owned() };

    // Pretty print json
    let r1: serde_json::Value = serde_json::from_str(&res1).unwrap();
    let r2: serde_json::Value = serde_json::from_str(&res2).unwrap();

    println!("============= INPUT 1 =============");
    if r1["isOk"].as_bool().unwrap() {
        println!("{}", r1["sql"].as_str().unwrap());
    } else {
        println!("Error: {}", r1["message"]);
    }

    println!("============= INPUT 2 =============");
    if r2["isOk"].as_bool().unwrap() {
        println!("{}", r2["sql"].as_str().unwrap());
    } else {
        println!("Error: {}", r2["message"]);
    }
}
