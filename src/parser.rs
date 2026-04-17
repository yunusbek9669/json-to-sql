use indexmap::IndexMap;
use serde_json::Value;
use regex::Regex;

use crate::models::{FilterRule, QueryNode, SourceDef};

static SOURCE_RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
    Regex::new(r"^([a-zA-Z0-9_]+)(?:\[(.*)\])?$").unwrap()
});

pub fn parse_source(source_str: &str) -> SourceDef {
    let caps = SOURCE_RE.captures(source_str).expect("Invalid @source format");
    let table_name = caps.get(1).map_or("", |m| m.as_str()).to_string();
    
    let mut filters = Vec::new();
    let mut limit = None;
    let mut offset = None;
    let mut order = None;
    let mut join_type = None;
    let mut rel = None;
    
    if let Some(rules_match) = caps.get(2) {
        let rules_str = rules_match.as_str().trim();
        if !rules_str.is_empty() {
            let parts: Vec<&str> = rules_str.split(',').collect();
            for part in parts {
                let part = part.trim();
                if part.is_empty() { continue; }
                
                if let Some((field, rest)) = part.split_once(':') {
                    let field = field.trim();
                    let rest = rest.trim();
                    
                    match field {
                        "$limit" => {
                            limit = rest.parse::<u64>().ok();
                        }
                        "$offset" => {
                            offset = rest.parse::<u64>().ok();
                        }
                        "$order" => {
                            order = Some(rest.to_string());
                        }
                        "$join" => {
                            join_type = Some(rest.to_lowercase());
                        }
                        "$rel" => {
                            rel = Some(rest.to_string());
                        }
                        _ => {
                            let (operator, value) = parse_operator_and_value(rest);
                            filters.push(FilterRule { field: field.to_string(), operator, value });
                        }
                    }
                }
            }
        }
    }
    
    SourceDef { table_name, filters, limit, offset, order, join_type, rel, from_macro: false }
}

fn parse_operator_and_value(input: &str) -> (String, String) {
    let input = input.trim();
    if input.starts_with("!:") {
        ("neq".to_string(), input[2..].trim().to_string())
    } else if input.starts_with(">") {
        ("gt".to_string(), input[1..].trim().to_string())
    } else if input.starts_with("<") {
        ("lt".to_string(), input[1..].trim().to_string())
    } else if input.starts_with("~") {
        ("like".to_string(), input[1..].trim().to_string())
    } else if input.starts_with("in ") || input.starts_with("IN ") {
        ("in".to_string(), input[2..].trim().to_string())
    } else if input.contains("..") {
        ("between".to_string(), input.to_string())
    } else {
        let val = if input.starts_with(':') {
            input[1..].trim().to_string()
        } else {
            input.to_string()
        };
        ("eq".to_string(), val)
    }
}

fn merge_json_objects(base: &mut serde_json::Map<String, Value>, override_map: &serde_json::Map<String, Value>) {
    for (k, v) in override_map {
        let base_val = base.remove(k);
        match (base_val, v) {
            (Some(Value::Object(mut base_obj)), Value::Object(override_obj)) => {
                merge_json_objects(&mut base_obj, override_obj);
                base.insert(k.clone(), Value::Object(base_obj));
            }
            (_, _) => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

pub fn parse_json(json_str: &str, external_macros: Option<&IndexMap<String, Value>>) -> Result<QueryNode, String> {
    let parsed: Value = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    if let Value::Object(map) = &parsed {
        let empty_macros = IndexMap::new();
        let macros = external_macros.unwrap_or(&empty_macros);
        if let Some(Value::Object(data_map)) = map.get("@data") {
            return parse_query_node("@data", data_map, macros, false, None);
        } else if let Some(Value::Object(data_list_map)) = map.get("@data[]") {
            return parse_query_node("@data[]", data_list_map, macros, false, None);
        }
        return Err("Root JSON must contain either '@data' or '@data[]'".to_string());
    }
    Err("Root JSON must be an object".to_string())
}

fn parse_query_node(name: &str, map: &serde_json::Map<String, Value>, macros: &IndexMap<String, Value>, in_macro: bool, inherited_join: Option<String>) -> Result<QueryNode, String> {
    let mut working_map = map.clone();
    let mut macro_overrides = None;
    let mut macro_name_opt = None;
    
    if let Some(Value::String(s)) = working_map.get("@source") {
        let base_name = s.split('[').next().unwrap_or(s).trim();
        if macros.contains_key(base_name) {
            macro_name_opt = Some(base_name.to_string());
            macro_overrides = Some(parse_source(s));
            working_map.remove("@source");
        }
    }
    
    let mut current_in_macro = in_macro;
    let mut current_inherited_join = inherited_join;

    if let Some(macro_name) = macro_name_opt {
        current_in_macro = true;
        if let Some(Value::Object(macro_def)) = macros.get(&macro_name) {
            let mut merged = macro_def.clone();
            merge_json_objects(&mut merged, &working_map);
            working_map = merged;
            if let Some(overrides) = &macro_overrides {
                if overrides.join_type.is_some() {
                    current_inherited_join = overrides.join_type.clone();
                }
            }
        } else {
            return Err(format!("Macro '{}' not found", macro_name));
        }
    }

    let (clean_name, is_list) = if name.ends_with("[]") {
        (name.trim_end_matches("[]").to_string(), true)
    } else {
        (name.to_string(), false)
    };
    
    let mut node = QueryNode {
        name: clean_name,
        is_list,
        source: None,
        flatten: false,
        fields: IndexMap::new(),
        children: Vec::new(),
        mode: None,
        from_macro: in_macro,
    };
    
    for (k, v) in &working_map {
        match k.as_str() {
            "@source" => {
                if let Value::String(s) = v {
                    let mut src = parse_source(s);
                    if let Some(overrides) = &macro_overrides {
                        // Merge macro overrides into the base source
                        src.filters.extend(overrides.filters.clone());
                        if overrides.limit.is_some() { src.limit = overrides.limit; }
                        if overrides.offset.is_some() { src.offset = overrides.offset; }
                        if overrides.order.is_some() { src.order = overrides.order.clone(); }
                        if overrides.join_type.is_some() { src.join_type = overrides.join_type.clone(); }
                        if overrides.rel.is_some() { src.rel = overrides.rel.clone(); }
                    }
                    
                    // Apply inherited join if no join_type is present on the source itself
                    if src.join_type.is_none() {
                        src.join_type = current_inherited_join.clone();
                    }
                    
                    src.from_macro = current_in_macro;
                    node.source = Some(src);
                }
            }
            "@mode" => { if let Value::String(s) = v { node.mode = Some(s.to_lowercase()); } }
            "@fields" => {
                if let Value::Object(fm) = v {
                    for (fk, fv) in fm { if let Value::String(fvs) = fv { node.fields.insert(fk.clone(), fvs.clone()); } }
                } else if let Value::Array(arr) = v {
                    for item in arr { if let Value::String(s) = item { node.fields.insert(s.clone(), s.clone()); } }
                }
            }
            "@flatten" => { if let Value::Bool(b) = v { node.flatten = *b; } }
            _ => {
                if !k.starts_with('@') {
                    if let Value::Object(child_map) = v {
                        let child = parse_query_node(k, child_map, macros, current_in_macro, current_inherited_join.clone())?;
                        node.children.push(child);
                    } else if let Value::Array(arr) = v {
                        let mut wrapper = QueryNode {
                            name: k.clone(),
                            is_list: k.ends_with("[]"),
                            source: None,
                            flatten: false,
                            fields: IndexMap::new(),
                            children: Vec::new(),
                            mode: None,
                            from_macro: in_macro,
                        };
                        for (idx, item) in arr.iter().enumerate() {
                            if let Value::Object(item_map) = item {
                                let child = parse_query_node(&idx.to_string(), item_map, macros, current_in_macro, current_inherited_join.clone())?;
                                wrapper.children.push(child);
                            }
                        }
                        node.children.push(wrapper);
                    }
                }
            }
        }
    }
    
    // Safety check: if macro_overrides were provided but no @source was found in macro (standalone structural macro),
    // we should still maybe do something, but typically macros have a @source.
    
    Ok(node)
}
