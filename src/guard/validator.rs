use regex::Regex;
use super::Guard;

impl Guard {
    pub fn validate_table(&self, context: &str) -> Result<(), String> {
        Self::check_global_threats(context)?;
        
        let re = Regex::new(r"^[a-zA-Z0-9_]+$").unwrap();
        if !re.is_match(context) {
            return Err(format!("Invalid table name format: {}", context));
        }

        // Check against whitelist if active (using the provided alias or real table name)
        if let Some(wl) = &self.whitelist {
            if !wl.contains_key(context) {
                return Err(format!("Table '{}' does not exist", context));
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
        
        let raw_field = if let Some((_, col)) = field.split_once('.') {
            col
        } else {
            field
        };

        if let Some(wl) = &self.whitelist {
            if let Some(rule) = wl.get(context) {
                if !rule.is_allowed(raw_field) {
                    return Err(format!("Column '{}' does not exist in table '{}'", raw_field, context));
                }
            } else {
                return Err(format!("Table '{}' does not exist", context));
            }
        }
        Ok(())
    }

    pub fn validate_field(&self, context: &str, field: &str, local_aliases: Option<&std::collections::HashMap<String, String>>) -> Result<(), String> {
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
            if let Some(rule) = wl.get(context) {
                if !rule.is_allowed("*") {
                    let re_str = Regex::new(r"'[^']*'").unwrap();
                    let field_no_str = re_str.replace_all(field, "");
                    let re_ident = Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]*").unwrap();
                    
                    for m in re_ident.find_iter(&field_no_str) {
                        let ident = m.as_str();
                        if builtins.contains(&ident.to_uppercase().as_str()) { continue; }
                        if ident.parse::<f64>().is_ok() { continue; }
                        
                        // If it's a locally aliased macro field, allow it
                        if let Some(aliases) = local_aliases {
                            if aliases.contains_key(ident) { continue; }
                        }
                        
                        // Strip potential prefix to match whitelist directly
                        let clean_ident = if let Some((_, col)) = ident.split_once('.') { col } else { ident };
                        if !rule.is_allowed(clean_ident) {
                            return Err(format!("Column '{}' does not exist in table '{}'", clean_ident, context));
                        }
                    }
                }
            } else {
                 return Err(format!("Table '{}' does not exist", context));
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
            if let Some(aliases) = local_aliases {
                if aliases.contains_key(field) {
                    return Ok(());
                }
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
}
