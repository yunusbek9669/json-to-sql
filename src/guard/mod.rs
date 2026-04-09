pub mod threats;
pub mod validator;
pub mod formatter;

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub enum WhitelistRule {
    /// ["id", "name", "*"] 
    Allowed(HashSet<String>),
    /// {"unique": "id", "full_name": "CONCAT(...)"}
    Mapping(HashMap<String, String>),
}

impl WhitelistRule {
    pub fn is_allowed(&self, field: &str) -> bool {
        match self {
            Self::Allowed(set) => set.contains("*") || set.contains(field),
            Self::Mapping(map) => map.contains_key("*") || map.contains_key(field),
        }
    }
    
    pub fn get_mapping(&self, field: &str) -> Option<String> {
        match self {
            Self::Mapping(map) => {
                if let Some(m) = map.get(field) {
                    Some(m.clone())
                } else if map.contains_key("*") {
                    Some(field.to_string())
                } else {
                    None
                }
            },
            Self::Allowed(set) => {
                if set.contains("*") || set.contains(field) { 
                    Some(field.to_string()) 
                } else { 
                    None 
                }
            },
        }
    }
}

pub struct Guard {
    /// alias -> WhitelistRule
    pub whitelist: Option<HashMap<String, WhitelistRule>>,
    /// alias -> real_table (e.g. "org" -> "structure_organization")
    pub alias_map: HashMap<String, String>,
    /// Set of real table names that have aliases (for enforcement)
    pub aliased_tables: HashSet<String>,
}

impl Guard {
    /// Parses whitelist keys with optional alias: "real_table:alias" -> columns
    /// Builds both whitelist (alias -> rule) and alias_map (alias -> real_table)
    pub fn new(raw_whitelist: Option<HashMap<String, serde_json::Value>>) -> Self {
        let mut alias_map = HashMap::new();
        let mut aliased_tables = HashSet::new();
        
        let whitelist = if let Some(raw) = raw_whitelist {
            let mut clean_whitelist = HashMap::new();
            for (key, val) in raw {
                let rule = if let Some(arr) = val.as_array() {
                    let mut set = HashSet::new();
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            set.insert(s.to_string());
                        }
                    }
                    WhitelistRule::Allowed(set)
                } else if let Some(obj) = val.as_object() {
                    let mut map = HashMap::new();
                    for (k, v) in obj {
                        if let Some(s) = v.as_str() {
                            map.insert(k.to_string(), s.to_string());
                        }
                    }
                    WhitelistRule::Mapping(map)
                } else {
                    WhitelistRule::Allowed(HashSet::new())
                };

                if let Some((real_table, alias)) = key.split_once(':') {
                    let real_table = real_table.trim().to_string();
                    let alias = alias.trim().to_string();
                    alias_map.insert(alias.clone(), real_table.clone());
                    aliased_tables.insert(real_table.clone());
                    clean_whitelist.insert(alias, rule);
                } else {
                    clean_whitelist.insert(key, rule);
                }
            }
            Some(clean_whitelist)
        } else {
            None
        };
        
        Self { 
            whitelist, 
            alias_map, 
            aliased_tables 
        }
    }

    /// Resolves an alias to a real table name.
    /// If the input is a real table name that has an alias, returns an error
    /// (frontend MUST use the alias, not the raw table name).
    pub fn resolve_alias(&self, name: &str) -> Result<String, String> {
        // Check if it's a valid alias
        if let Some(real) = self.alias_map.get(name) {
            return Ok(real.clone());
        }
        
        // If it's a real table name that should be aliased, block it
        if self.aliased_tables.contains(name) {
            return Err(format!("Table '{}' does not exist", name));
        }
        
        // No alias exists — use the name directly
        Ok(name.to_string())
    }
}
