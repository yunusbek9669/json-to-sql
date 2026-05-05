use indexmap::IndexMap;
use serde_json::{json, Value};

use crate::guard::Guard;
use crate::parser::parse_source;

/// Entry point. `op_value` is either a JSON object or a JSON array.
pub fn process_operation(
    op_value: &Value,
    raw_whitelist: Option<IndexMap<String, Value>>,
) -> Value {
    let guard = Guard::new(raw_whitelist);
    let mut updates: Vec<Value> = Vec::new();
    let mut inserts: Vec<Value> = Vec::new();
    let mut rejected: Vec<String> = Vec::new();

    let entries: Vec<(&str, &serde_json::Map<String, Value>)> = match op_value {
        Value::Object(map) => map
            .iter()
            .filter_map(|(k, v)| v.as_object().map(|obj| (k.as_str(), obj)))
            .collect(),
        Value::Array(arr) => arr
            .iter()
            .flat_map(|item| {
                item.as_object()
                    .into_iter()
                    .flat_map(|obj| obj.iter().filter_map(|(k, v)| v.as_object().map(|o| (k.as_str(), o))))
            })
            .collect(),
        _ => {
            return json!({
                "isOk": false,
                "data": null,
                "rejected": [],
                "message": "@operation must be an object or array"
            });
        }
    };

    for (source_str, fields_obj) in entries {
        if let Err(e) = process_entry(source_str, fields_obj, &guard, &mut updates, &mut inserts, &mut rejected) {
            return json!({
                "isOk": false,
                "data": null,
                "rejected": rejected,
                "message": e
            });
        }
    }

    json!({
        "isOk": true,
        "data": {
            "update": updates,
            "insert": inserts
        },
        "rejected": rejected,
        "message": "success"
    })
}

fn process_entry(
    source_str: &str,
    fields_obj: &serde_json::Map<String, Value>,
    guard: &Guard,
    updates: &mut Vec<Value>,
    inserts: &mut Vec<Value>,
    rejected: &mut Vec<String>,
) -> Result<(), String> {
    let source = parse_source(source_str);
    let table_alias = &source.table_name;

    let real_table = guard.resolve_alias(table_alias)?;

    let wl_rule = guard.whitelist.as_ref().and_then(|wl| wl.get(table_alias.as_str()));

    let mut mapped_fields: IndexMap<String, Value> = IndexMap::new();
    for (virtual_col, field_val) in fields_obj {
        let real_col = if let Some(rule) = wl_rule {
            rule.get_mapping(virtual_col)
        } else {
            Some(virtual_col.clone())
        };

        match real_col {
            None => rejected.push(virtual_col.clone()),
            Some(real_col_name) => {
                if let Value::String(s) = field_val {
                    Guard::check_global_threats(s).map_err(|e| {
                        format!("Threat detected in field '{}': {}", virtual_col, e)
                    })?;
                }
                mapped_fields.insert(real_col_name, field_val.clone());
            }
        }
    }

    if !source.filters.is_empty() {
        let mut filter_map: IndexMap<String, Value> = IndexMap::new();
        for filter in &source.filters {
            let real_filter_col = if let Some(rule) = wl_rule {
                rule.get_mapping(&filter.field).unwrap_or_else(|| filter.field.clone())
            } else {
                filter.field.clone()
            };
            Guard::check_global_threats(&filter.value).map_err(|e| {
                format!("Threat detected in filter '{}': {}", filter.field, e)
            })?;
            filter_map.insert(real_filter_col, coerce_value(&filter.value));
        }
        let mut entry = serde_json::Map::new();
        entry.insert("filter".to_string(), json!(filter_map));
        entry.insert(real_table, json!(mapped_fields));
        updates.push(Value::Object(entry));
    } else {
        let mut entry = serde_json::Map::new();
        entry.insert(real_table, json!(mapped_fields));
        inserts.push(Value::Object(entry));
    }

    Ok(())
}

fn coerce_value(s: &str) -> Value {
    if let Ok(n) = s.parse::<i64>() { return json!(n); }
    if let Ok(f) = s.parse::<f64>() { return json!(f); }
    Value::String(s.to_string())
}
