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
                    
                    // Handle $limit, $order, $offset directives
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
                        _ => {
                            let (operator, value) = parse_operator_and_value(rest);
                            filters.push(FilterRule { field: field.to_string(), operator, value });
                        }
                    }
                }
            }
        }
    }
    
    SourceDef { table_name, filters, limit, offset, order }
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
/// Supports both new compact format (root is the node itself) and
/// legacy format with @data/@config wrappers.
pub fn parse_json(json_str: &str) -> Result<QueryNode, String> {
    let parsed: Value = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    
    if let Value::Object(map) = &parsed {
        // Check if this is legacy format with @data wrapper
        if let Some(Value::Object(data_map)) = map.get("@data") {
            // Legacy format: merge @config into the @data node's @source
            let mut root_map = data_map.clone();
            
            // If there's a top-level @config, merge $limit/$order/$offset into @source
            if let Some(Value::Object(cfg)) = map.get("@config") {
                if let Some(Value::String(source_str)) = root_map.get("@source") {
                    let mut new_source = source_str.clone();
                    let mut extras = Vec::new();
                    
                    if let Some(Value::Number(l)) = cfg.get("limit") {
                        extras.push(format!("$limit: {}", l));
                    }
                    if let Some(Value::Number(o)) = cfg.get("offset") {
                        extras.push(format!("$offset: {}", o));
                    }
                    if let Some(Value::String(o)) = cfg.get("order") {
                        extras.push(format!("$order: {}", o));
                    }
                    
                    if !extras.is_empty() {
                        // Inject into existing brackets or add new ones
                        if new_source.contains('[') {
                            // Insert before closing bracket
                            let close_pos = new_source.rfind(']').unwrap();
                            new_source.insert_str(close_pos, &format!(", {}", extras.join(", ")));
                        } else {
                            new_source.push_str(&format!("[{}]", extras.join(", ")));
                        }
                        root_map.insert("@source".to_string(), Value::String(new_source));
                    }
                }
            }
            
            return parse_query_node("@data", &root_map);
        }
        
        // New compact format: root JSON IS the node
        return parse_query_node("@root", map);
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
    };
    
    for (k, v) in map {
        match k.as_str() {
            "@source" => {
                if let Value::String(s) = v {
                    node.source = Some(parse_source(s));
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
                    }
                }
            }
        }
    }
    Ok(node)
}
