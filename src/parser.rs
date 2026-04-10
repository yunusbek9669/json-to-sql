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
                    
                    // Handle $limit, $order, $offset, $join, $rel directives
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
    
    SourceDef { table_name, filters, limit, offset, order, join_type, rel }
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

/// Parses the top-level JSON into a single root QueryNode.
pub fn parse_json(json_str: &str) -> Result<QueryNode, String> {
    let parsed: Value = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    
    if let Value::Object(map) = &parsed {
        // Enforce @data or @data[]
        if let Some(Value::Object(data_map)) = map.get("@data") {
            return parse_query_node("@data", data_map);
        } else if let Some(Value::Object(data_list_map)) = map.get("@data[]") {
            return parse_query_node("@data[]", data_list_map);
        } else if map.contains_key("@info") {
            return Err("Expected @data or @data[], but got @info. Info requests should be handled earlier.".to_string());
        }
        
        return Err("Root JSON must contain either '@data' or '@data[]' as the primary key".to_string());
    }
    
    Err("Root JSON must be an object".to_string())
}

fn parse_query_node(name: &str, map: &serde_json::Map<String, Value>) -> Result<QueryNode, String> {
    // Detect [] suffix for list nodes
    let (clean_name, is_list) = if name.ends_with("[]") {
        (name.trim_end_matches("[]").to_string(), true)
    } else {
        (name.to_string(), false)
    };
    
    let mut node = QueryNode {
        name: clean_name,
        is_list,
        source: None,
        join: None,
        flatten: false,
        fields: IndexMap::new(),
        children: Vec::new(),
        mode: None,
    };
    
    for (k, v) in map {
        match k.as_str() {
            "@source" => {
                if let Value::String(s) = v {
                    node.source = Some(parse_source(s));
                }
            }
            "@mode" => {
                if let Value::String(s) = v {
                    node.mode = Some(s.to_lowercase());
                }
            }
            "@join" => {
                if let Value::String(j) = v {
                    node.join = Some(j.clone());
                }
            }
            "@fields" => {
                if let Value::Object(fm) = v {
                    for (fk, fv) in fm {
                        if let Value::String(fvs) = fv {
                            node.fields.insert(fk.clone(), fvs.clone());
                        }
                    }
                } else if let Value::Array(arr) = v {
                    for item in arr {
                        if let Value::String(s) = item {
                            node.fields.insert(s.clone(), s.clone());
                        }
                    }
                }
            }
            "@flatten" => {
                if let Value::Bool(b) = v {
                    node.flatten = *b;
                }
            }
            _ => {
                if !k.starts_with('@') {
                    if let Value::Object(child_map) = v {
                        let child = parse_query_node(k, child_map)?;
                        node.children.push(child);
                    } else if let Value::Array(arr) = v {
                        // Create a structural wrapper node for the array
                        let mut wrapper = QueryNode {
                            name: k.clone(),
                            is_list: k.ends_with("[]"), // Preserve list mode if requested
                            source: None,
                            join: None,
                            flatten: false,
                            fields: IndexMap::new(),
                            children: Vec::new(),
                            mode: None,
                        };
                        
                        // Parse each object in the array as a child with a numeric name
                        for (idx, item) in arr.iter().enumerate() {
                            if let Value::Object(item_map) = item {
                                let child_name = idx.to_string();
                                let child = parse_query_node(&child_name, item_map)?;
                                wrapper.children.push(child);
                            }
                        }
                        node.children.push(wrapper);
                    }
                }
            }
        }
    }
    Ok(node)
}
