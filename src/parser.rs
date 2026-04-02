use std::collections::HashMap;
use serde_json::Value;
use regex::Regex;

use crate::models::{FilterRule, GlobalConfig, QueryNode, RootQuery, SourceDef};

static SOURCE_RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
    Regex::new(r"^([a-zA-Z0-9_]+)(?:\[(.*)\])?$").unwrap()
});

pub fn parse_source(source_str: &str) -> SourceDef {
    let caps = SOURCE_RE.captures(source_str).expect("Invalid @source format");
    let table_name = caps.get(1).map_or("", |m| m.as_str()).to_string();
    
    let mut filters = Vec::new();
    
    if let Some(rules_match) = caps.get(2) {
        let rules_str = rules_match.as_str().trim();
        if !rules_str.is_empty() {
            // A simple split by comma, though this might break if quotes contain commas.
            // For now, assume commas in values are not unescaped, or we split properly.
            // In a real robust parser, we'd use pest or nom. For simplicity of the spec, a regex split is step 1.
            let parts: Vec<&str> = rules_str.split(',').collect();
            for part in parts {
                let part = part.trim();
                if part.is_empty() { continue; }
                
                // e.g. "status: 'active'", "age: 25..45", "rank_id: in (1, 2, 3)"
                if let Some((field, rest)) = part.split_once(':') {
                    let field = field.trim().to_string();
                    let rest = rest.trim();
                    let (operator, value) = parse_operator_and_value(rest);
                    filters.push(FilterRule { field, operator, value });
                }
            }
        }
    }
    
    SourceDef { table_name, filters }
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
        // "25..45"
        ("between".to_string(), input.to_string())
    } else {
        // default to eq
        // "active" or ":active" (wait, if they use `!:` it was above, but if they just use `:` as part of `field: value` we just have value)
        let val = if input.starts_with(':') {
            input[1..].trim().to_string()
        } else {
            input.to_string()
        };
        ("eq".to_string(), val)
    }
}

pub fn parse_json(json_str: &str) -> Result<RootQuery, String> {
    let parsed: Value = serde_json::from_str(json_str).map_err(|e| e.to_string())?;
    
    let mut config = GlobalConfig::default();
    let mut nodes = Vec::new();
    
    if let Value::Object(map) = parsed {
        for (k, v) in map {
            if k == "@config" {
                if let Value::Object(cfg) = v {
                    if let Some(Value::Number(l)) = cfg.get("limit") {
                        config.limit = l.as_u64();
                    }
                    if let Some(Value::Number(o)) = cfg.get("offset") {
                        config.offset = o.as_u64();
                    }
                    if let Some(Value::String(o)) = cfg.get("order") {
                        config.order = Some(o.clone());
                    }
                }
            } else {
                if let Value::Object(node_map) = v {
                    let node = parse_query_node(&k, &node_map)?;
                    nodes.push(node);
                }
            }
        }
    } else {
        return Err("Root JSON must be an object".to_string());
    }
    
    Ok(RootQuery { nodes, config })
}

fn parse_query_node(name: &str, map: &serde_json::Map<String, Value>) -> Result<QueryNode, String> {
    let mut node = QueryNode {
        name: name.to_string(),
        source: None,
        join: None,
        fields: HashMap::new(),
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
            _ => {
                // Ignore other @ config keys not handled yet or parse as child nodes
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
