use regex::Regex;
use std::collections::{HashMap, HashSet};

pub struct Guard {
    /// real_table -> allowed_columns
    pub whitelist: Option<HashMap<String, Vec<String>>>,
    /// alias -> real_table (e.g. "org" -> "structure_organization")
    pub alias_map: HashMap<String, String>,
    /// Set of real table names that have aliases (for enforcement)
    pub aliased_tables: HashSet<String>,
}

impl Guard {
    /// Parses whitelist keys with optional alias: "real_table:alias" -> columns
    /// Builds both whitelist (real_table -> columns) and alias_map (alias -> real_table)
    pub fn new(raw_whitelist: Option<HashMap<String, Vec<String>>>) -> Self {
        let mut alias_map = HashMap::new();
        let mut aliased_tables = HashSet::new();
        
        let whitelist = if let Some(raw) = raw_whitelist {
            let mut clean_whitelist = HashMap::new();
            for (key, columns) in raw {
                if let Some((real_table, alias)) = key.split_once(':') {
                    let real_table = real_table.trim().to_string();
                    let alias = alias.trim().to_string();
                    alias_map.insert(alias.clone(), real_table.clone());
                    aliased_tables.insert(real_table.clone());
                    // Use alias as the key in whitelist for better security context
                    clean_whitelist.insert(alias, columns);
                } else {
                    // No alias — table name is used directly as context
                    clean_whitelist.insert(key, columns);
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
            return Err(format!(
                "Table '{}' is strictly prohibited by whitelist",
                name
            ));
        }
        
        // No alias exists — use the name directly
        Ok(name.to_string())
    }

    pub fn check_global_threats(input: &str) -> Result<(), String> {
        let upper = input.to_uppercase();
        
        // Block comment and logic terminators
        if input.contains("--") || input.contains("/*") || input.contains("*/") || input.contains(";") {
            return Err(format!("Comment sequences or terminators are strictly forbidden. Input: {}", input));
        }

        // Block structural manipulation and multi-queries
        let bad_words = [
            "DROP ", "DELETE ", "UPDATE ", "INSERT ", "EXEC ", "TRUNCATE ", 
            "ALTER ", "GRANT ", "REVOKE ", "UNION "
        ];
        
        for word in bad_words {
            if upper.contains(word) {
                return Err(format!("Forbidden SQL operation detected: {}", word));
            }
        }
        
        Ok(())
    }

    pub fn validate_table(&self, context: &str) -> Result<(), String> {
        Self::check_global_threats(context)?;
        
        let re = Regex::new(r"^[a-zA-Z0-9_]+$").unwrap();
        if !re.is_match(context) {
            return Err(format!("Invalid table name format: {}", context));
        }

        // Check against whitelist if active (using the provided alias or real table name)
        if let Some(wl) = &self.whitelist {
            if !wl.contains_key(context) {
                return Err(format!("Table '{}' is strictly prohibited by whitelist", context));
            }
        }

        Ok(())
    }

    pub fn validate_column(&self, context: &str, field: &str) -> Result<(), String> {
        Self::check_global_threats(field)?;
        
        let re = Regex::new(r"^[a-zA-Z0-9_\.]+$").unwrap();
        if !re.is_match(field) {
            return Err(format!("Invalid identifier format: {} in table {}", field, context));
        }
        
        // Strip out table prefix if present, e.g. "personal.id" -> "id"
        let raw_field = if let Some((_, col)) = field.split_once('.') {
            col
        } else {
            field
        };

        if let Some(wl) = &self.whitelist {
            if let Some(allowed) = wl.get(context) {
                if !allowed.contains(&"*".to_string()) && !allowed.contains(&raw_field.to_string()) {
                    return Err(format!("Column '{}' is not on the whitelist for table '{}'", raw_field, context));
                }
            } else {
                return Err(format!("Table '{}' is missing from whitelist context", context));
            }
        }
        Ok(())
    }

    pub fn validate_field(&self, context: &str, field: &str) -> Result<(), String> {
        Self::check_global_threats(field)?;
        let field_upper = field.trim().to_uppercase();
        
        let builtins = vec![
            "CONCAT", "CONCAT_WS", "SUBSTR", "SUBSTRING", "LEFT", "RIGHT", "REPLACE", "UPPER", "LOWER", 
            "TRIM", "LTRIM", "RTRIM", "LENGTH", "CHAR_LENGTH", "POSITION", "COUNT", "SUM", "AVG", "MAX", 
            "MIN", "COALESCE", "NULLIF", "GREATEST", "LEAST", "DATE_FORMAT", "TO_CHAR", "TO_TIMESTAMP", 
            "TO_DATE", "NOW", "CURRENT_DATE", "CURRENT_TIMESTAMP", "CURRENT_TIME", "DATE_TRUNC", "EXTRACT", 
            "AGE", "CAST", "ROUND", "CEIL", "FLOOR", "ABS", "POWER", "SQRT", "MOD", "SIGN", "SPLIT_PART", 
            "JSON_EXTRACT_PATH_TEXT", "JSONB_EXTRACT_PATH_TEXT", "CASE", "WHEN", "THEN", "ELSE", "END", 
            "AS", "IN", "IS", "NULL", "AND", "OR", "NOT", "TRUE", "FALSE"
        ];

        // Ensure identifiers inside the expression exist in the whitelist
        if let Some(wl) = &self.whitelist {
            if let Some(allowed) = wl.get(context) {
                if !allowed.contains(&"*".to_string()) {
                    let re_str = Regex::new(r"'[^']*'").unwrap();
                    let field_no_str = re_str.replace_all(field, "");
                    let re_ident = Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]*").unwrap();
                    
                    for m in re_ident.find_iter(&field_no_str) {
                        let ident = m.as_str();
                        if builtins.contains(&ident.to_uppercase().as_str()) { continue; }
                        if ident.parse::<f64>().is_ok() { continue; }
                        
                        // Strip potential prefix to match whitelist directly
                        let clean_ident = if let Some((_, col)) = ident.split_once('.') { col } else { ident };
                        if !allowed.contains(&clean_ident.to_string()) {
                            return Err(format!("Expression column '{}' is blocked for table '{}'", clean_ident, context));
                        }
                    }
                }
            } else {
                 return Err(format!("Table '{}' is blocked", context));
            }
        }

        // 1. Allow CASE expressions directly
        if field_upper.starts_with("CASE ") && field_upper.ends_with(" END") {
            if field_upper.contains("SELECT ") {
                return Err("Unsafe CASE expression with SELECT".to_string());
            }
            return Ok(());
        }

        // 2. Allow Function Calls
        if field.contains('(') {
            let func_name = field.split('(').next().unwrap_or("").trim().to_uppercase();
            if !builtins.contains(&func_name.as_str()) {
                return Err(format!("Unsafe or unsupported function call: {}", func_name));
            }
            if field_upper.contains("SELECT ") {
                return Err("Subqueries are not allowed inside functions".to_string());
            }
        } else {
            // 3. Allow constants or identifiers
            if (field.starts_with('\'') && field.ends_with('\'')) || field.parse::<f64>().is_ok() {
                return Ok(());
            }
            self.validate_column(context, field)?; 
        }
        Ok(())
    }

    pub fn is_safe_order_by(&self, order: &str) -> Result<(), String> {
        Self::check_global_threats(order)?;
        let parts: Vec<&str> = order.split_whitespace().collect();
        if parts.is_empty() || parts.len() > 2 {
            return Err("Invalid ORDER BY format".to_string());
        }
        
        let re = Regex::new(r"^[a-zA-Z0-9_\.]+$").unwrap();
        if !re.is_match(parts[0]) {
            return Err("Invalid ORDER BY identifier format".to_string());
        }
        
        if parts.len() == 2 {
            let dir = parts[1].to_uppercase();
            if dir != "ASC" && dir != "DESC" {
                return Err("ORDER BY direction must be ASC or DESC".to_string());
            }
        }
        Ok(())
    }

    pub fn auto_prefix_field(field_sql: &str, table_alias: &str) -> String {
        let builtins = vec![
            "CONCAT", "CONCAT_WS", "SUBSTR", "SUBSTRING", "LEFT", "RIGHT", "REPLACE", "UPPER", "LOWER", 
            "TRIM", "LTRIM", "RTRIM", "LENGTH", "CHAR_LENGTH", "POSITION", "COUNT", "SUM", "AVG", "MAX", 
            "MIN", "COALESCE", "NULLIF", "GREATEST", "LEAST", "DATE_FORMAT", "TO_CHAR", "TO_TIMESTAMP", 
            "TO_DATE", "NOW", "CURRENT_DATE", "CURRENT_TIMESTAMP", "CURRENT_TIME", "DATE_TRUNC", "EXTRACT", 
            "AGE", "CAST", "ROUND", "CEIL", "FLOOR", "ABS", "POWER", "SQRT", "MOD", "SIGN", "SPLIT_PART", 
            "JSON_EXTRACT_PATH_TEXT", "JSONB_EXTRACT_PATH_TEXT", "CASE", "WHEN", "THEN", "ELSE", "END", 
            "AS", "IN", "IS", "NULL", "AND", "OR", "NOT", "TRUE", "FALSE", "YEAR", "MONTH", "DAY", "HOUR", 
            "MINUTE", "SECOND", "FROM", "INTERVAL", "JSON_BUILD_OBJECT",
            "DESC", "ASC", "NULLS", "FIRST", "LAST", "BETWEEN", "LIKE", "ILIKE", "DISTINCT", "ALL", "ANY",
            "EXISTS", "HAVING", "GROUP", "BY", "ORDER", "LIMIT", "OFFSET", "UNION", "INTERSECT", "EXCEPT",
            "FILTER", "OVER", "PARTITION", "WINDOW", "ROWS", "RANGE", "UNBOUNDED", "PRECEDING", "FOLLOWING",
            "MALE", "FEMALE"
        ];

        let mut result = String::with_capacity(field_sql.len() + 20);
        let mut in_quotes = false;
        let mut current_word = String::new();
        
        let chars: Vec<char> = field_sql.chars().collect();
        let mut i = 0;
        
        while i < chars.len() {
            let c = chars[i];
            
            if c == '\'' {
                in_quotes = !in_quotes;
                if !current_word.is_empty() {
                    result.push_str(&current_word);
                    current_word.clear();
                }
                result.push(c);
            } else if !in_quotes && (c.is_alphanumeric() || c == '_') {
                current_word.push(c);
            } else {
                if !current_word.is_empty() {
                    let is_numeric = current_word.parse::<f64>().is_ok();
                    let is_builtin = builtins.contains(&current_word.to_uppercase().as_str());
                    let has_dot_after = c == '.';
                    let has_dot_before = result.ends_with('.');
                    
                    if !is_numeric && !is_builtin && !has_dot_after && !has_dot_before {
                        result.push_str(&format!("{}.{}", table_alias, current_word));
                    } else {
                        result.push_str(&current_word);
                    }
                    current_word.clear();
                }
                result.push(c);
            }
            i += 1;
        }
        
        if !current_word.is_empty() {
            let is_numeric = current_word.parse::<f64>().is_ok();
            let is_builtin = builtins.contains(&current_word.to_uppercase().as_str());
            let has_dot_before = result.ends_with('.');
            
            if !is_numeric && !is_builtin && !has_dot_before {
                result.push_str(&format!("{}.{}", table_alias, current_word));
            } else {
                result.push_str(&current_word);
            }
        }
        
        result
    }
}
