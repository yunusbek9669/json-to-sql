use regex::Regex;
use std::collections::HashMap;

pub struct Guard {
    pub whitelist: Option<HashMap<String, Vec<String>>>,
}

impl Guard {
    pub fn new(whitelist: Option<HashMap<String, Vec<String>>>) -> Self {
        Self { whitelist }
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

    pub fn validate_table(&self, name: &str) -> Result<(), String> {
        Self::check_global_threats(name)?;
        
        let re = Regex::new(r"^[a-zA-Z0-9_]+$").unwrap();
        if !re.is_match(name) {
            return Err(format!("Invalid table name format: {}", name));
        }

        // Check against whitelist if active
        if let Some(wl) = &self.whitelist {
            if !wl.contains_key(name) {
                return Err(format!("Table '{}' is strictly prohibited by whitelist", name));
            }
        }

        Ok(())
    }

    pub fn validate_column(&self, table: &str, field: &str) -> Result<(), String> {
        Self::check_global_threats(field)?;
        
        let re = Regex::new(r"^[a-zA-Z0-9_\.]+$").unwrap();
        if !re.is_match(field) {
            return Err(format!("Invalid identifier format: {}", field));
        }
        
        // Strip out table prefix if present, e.g. "personal.id" -> "id"
        let raw_field = if let Some((_, col)) = field.split_once('.') {
            col
        } else {
            field
        };

        if let Some(wl) = &self.whitelist {
            if let Some(allowed) = wl.get(table) {
                if !allowed.contains(&"*".to_string()) && !allowed.contains(&raw_field.to_string()) {
                    return Err(format!("Column '{}' is not on the whitelist for table '{}'", raw_field, table));
                }
            } else {
                return Err(format!("Table '{}' is missing from whitelist context", table));
            }
        }
        Ok(())
    }

    pub fn validate_field(&self, table: &str, field: &str) -> Result<(), String> {
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
            if let Some(allowed) = wl.get(table) {
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
                            return Err(format!("Expression column '{}' is blocked for table '{}'", clean_ident, table));
                        }
                    }
                }
            } else {
                 return Err(format!("Table '{}' is blocked", table));
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
            self.validate_column(table, field)?; 
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
}
