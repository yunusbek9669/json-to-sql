pub mod threats;
pub mod validator;
pub mod formatter;

use std::collections::{HashMap, HashSet};
use indexmap::{IndexMap, IndexSet};
use crate::models::FilterRule;

#[derive(Debug, Clone)]
pub enum WhitelistRule {
    /// ["id", "name", "*"] 
    Allowed(IndexSet<String>),
    /// {"unique": "id", "full_name": "CONCAT(...)"}
    Mapping(IndexMap<String, String>),
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
    pub whitelist: Option<IndexMap<String, WhitelistRule>>,
    /// alias -> real_table (e.g. "org" -> "structure_organization")
    pub alias_map: HashMap<String, String>,
    /// Set of real table names that have aliases (for enforcement)
    pub aliased_tables: HashSet<String>,
    /// alias -> backend-defined default filters (from whitelist key syntax "table[status:1]")
    pub default_filters: HashMap<String, Vec<FilterRule>>,
    /// alias -> backend-defined default ORDER BY (from whitelist key syntax "table[$order: col ASC]")
    pub default_order: HashMap<String, String>,
}

impl Guard {
    /// Parses whitelist keys with optional alias and optional default filters.
    ///
    /// Supported key formats:
    ///   "real_table"                   → no alias, no default filters
    ///   "real_table:alias"             → with alias, no default filters
    ///   "real_table[status:1]"         → no alias, with backend-defined default filter
    ///   "real_table:alias[status:1]"   → with alias and backend-defined default filter
    ///
    /// Default filters (inside `[...]`) are applied automatically to every query that uses
    /// the table. The filtered columns do NOT need to appear in the whitelist column list
    /// because these conditions are backend-defined, not user-supplied.
    pub fn new(raw_whitelist: Option<IndexMap<String, serde_json::Value>>) -> Self {
        let mut alias_map = HashMap::new();
        let mut aliased_tables = HashSet::new();
        let mut default_filters: HashMap<String, Vec<FilterRule>> = HashMap::new();
        let mut default_order: HashMap<String, String> = HashMap::new();

        let whitelist = if let Some(raw) = raw_whitelist {
            let mut clean_whitelist = IndexMap::new();
            for (key, val) in raw {
                let rule = if let Some(arr) = val.as_array() {
                    let mut set = IndexSet::new();
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            set.insert(s.to_string());
                        }
                    }
                    WhitelistRule::Allowed(set)
                } else if let Some(obj) = val.as_object() {
                    let mut map = IndexMap::new();
                    for (k, v) in obj {
                        if let Some(s) = v.as_str() {
                            map.insert(k.to_string(), s.to_string());
                        }
                    }
                    WhitelistRule::Mapping(map)
                } else {
                    WhitelistRule::Allowed(IndexSet::new())
                };

                // Extract optional "[filter]" suffix from the key
                let (base_key, filter_str) = if let Some(bracket_pos) = key.find('[') {
                    let filter_content = if key.ends_with(']') {
                        key[bracket_pos + 1..key.len() - 1].trim().to_string()
                    } else {
                        key[bracket_pos + 1..].trim().to_string()
                    };
                    (key[..bracket_pos].trim().to_string(), Some(filter_content))
                } else {
                    (key.trim().to_string(), None)
                };

                // Parse optional ":alias" from base_key
                let effective_key = if let Some((real_table, alias)) = base_key.split_once(':') {
                    let real_table = real_table.trim().to_string();
                    let alias = alias.trim().to_string();
                    alias_map.insert(alias.clone(), real_table.clone());
                    aliased_tables.insert(real_table.clone());
                    alias
                } else {
                    base_key
                };

                // Parse and store default filters / order if present
                if let Some(fs) = filter_str {
                    if !fs.is_empty() {
                        if let Some(ord) = parse_default_order_param(&fs) {
                            default_order.insert(effective_key.clone(), ord);
                        }
                        let filters = parse_default_filters(&fs);
                        if !filters.is_empty() {
                            default_filters.insert(effective_key.clone(), filters);
                        }
                    }
                }

                clean_whitelist.insert(effective_key, rule);
            }
            Some(clean_whitelist)
        } else {
            None
        };

        Self {
            whitelist,
            alias_map,
            aliased_tables,
            default_filters,
            default_order,
        }
    }

    /// Returns the default filters for a given alias (or empty slice if none).
    pub fn get_default_filters(&self, alias: &str) -> &[FilterRule] {
        self.default_filters.get(alias).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Returns the default ORDER BY for a given alias, if set in the whitelist key.
    /// User-supplied `$order` in `@source` always takes precedence over this.
    pub fn get_default_order(&self, alias: &str) -> Option<&str> {
        self.default_order.get(alias).map(|s| s.as_str())
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

/// Extracts `$order` value from a whitelist filter string, e.g. "status:1, $order: col ASC".
/// Returns `None` if absent or value is not safe.
fn parse_default_order_param(filter_str: &str) -> Option<String> {
    for part in split_filter_args(filter_str) {
        if let Some((field, rest)) = part.split_once(':') {
            if field.trim() == "$order" {
                let val = rest.trim();
                // Allow only word chars, dots, and optional trailing ASC/DESC
                if !val.is_empty()
                    && val.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == ' ')
                {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// Parses a filter string like "status:1, type!:0" into FilterRule vec.
/// Used for backend-defined default filters in whitelist keys.
fn parse_default_filters(filter_str: &str) -> Vec<FilterRule> {
    let mut filters = Vec::new();
    for part in split_filter_args(filter_str) {
        if let Some((field, rest)) = part.split_once(':') {
            let field = field.trim();
            if field.is_empty() || field.starts_with('$') {
                continue;
            }
            let rest = rest.trim();
            let (operator, value) = parse_filter_value(rest);
            filters.push(FilterRule { field: field.to_string(), operator, value });
        }
    }
    filters
}

/// Splits a filter string by top-level commas (ignoring commas inside `(...)` or `[...]`).
fn split_filter_args(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '(' | '[' => { depth += 1; current.push(c); }
            ')' | ']' => { depth -= 1; current.push(c); }
            ',' if depth == 0 => {
                let t = current.trim().to_string();
                if !t.is_empty() { parts.push(t); }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    let t = current.trim().to_string();
    if !t.is_empty() { parts.push(t); }
    parts
}

/// Parses the value portion of a filter rule (everything after the first `:`).
fn parse_filter_value(input: &str) -> (String, String) {
    let input = input.trim();
    if input.eq_ignore_ascii_case("null") {
        ("is_null".to_string(), String::new())
    } else if input.eq_ignore_ascii_case("!null") {
        ("is_not_null".to_string(), String::new())
    } else if input.starts_with("!:") {
        ("neq".to_string(), input[2..].trim().to_string())
    } else if input.starts_with('>') {
        ("gt".to_string(), input[1..].trim().to_string())
    } else if input.starts_with('<') {
        ("lt".to_string(), input[1..].trim().to_string())
    } else if input.starts_with('~') {
        ("like".to_string(), input[1..].trim().to_string())
    } else if input.to_ascii_lowercase().starts_with("in ") {
        ("in".to_string(), input[3..].trim().to_string())
    } else if input.contains("..") {
        ("between".to_string(), input.to_string())
    } else {
        let val = if input.starts_with(':') { input[1..].trim().to_string() } else { input.to_string() };
        ("eq".to_string(), val)
    }
}
