use json_to_sql::parser;
use serde_json::json;

fn main() {
    let j = json!({
        "@data": {
            "optional ky name": [
                { "@source": "org" },
                { "@source": "innerOrg" }
            ]
        }
    });
    let s = serde_json::to_string(&j).unwrap();
    let root = parser::parse_json(&s);
    println!("{:#?}", root.unwrap().children.first().unwrap().children);
}
