use std::collections::HashMap;

fn main() {
    let json = serde_json::json!({
      "input1": {
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
      },
      "input2": {
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
      },
      "whitelist": {
        "emp": ["*"],
        "departmentStaffPosition": ["*"],
        "org": ["*"],
        "innerOrg": ["*"],
        "departmentBasic": ["*"],
        "education": ["*"]
      },
      "relations": {
        "emp->departmentStaffPosition": "LEFT JOIN @table ON @1.id = @2.emp_id",
        "departmentStaffPosition->org": "LEFT JOIN @table ON @1.org_id = @2.id",
        "departmentStaffPosition->innerOrg": "LEFT JOIN @table ON @1.inner_org_id = @2.id",
        "departmentStaffPosition->departmentBasic": "LEFT JOIN @table ON @1.dep_id = @2.id",
        "emp->education": "LEFT JOIN @table ON @1.id = @2.emp_id"
      },
      "macros": {
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
    });

    // Write to a temporary rust project or use the library directly.
    // Let's write a small script inside the json-to-sql project
}
