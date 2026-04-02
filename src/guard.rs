use regex::Regex;

pub struct Guard;

impl Guard {
    pub fn validate_table(name: &str) -> Result<(), String> {
        let re = Regex::new(r"^[a-zA-Z0-9_]+$").unwrap();
        if !re.is_match(name) {
            return Err(format!("Invalid table name: {}", name));
        }
        Ok(())
    }

    pub fn validate_identifier(name: &str) -> Result<(), String> {
        // e.g., allow basic characters, dots
        let re = Regex::new(r"^[a-zA-Z0-9_\.]+$").unwrap();
        if !re.is_match(name) {
            return Err(format!("Invalid identifier: {}", name));
        }
        Ok(())
    }

    pub fn validate_field(field: &str) -> Result<(), String> {
        // Simple heuristic: if it contains an open paren, it must start with a whitelisted function
        if field.contains('(') {
            let func_name = field.split('(').next().unwrap_or("").trim().to_uppercase();
            let safe_funcs = vec!["CONCAT", "COUNT", "SUM", "AVG", "MAX", "MIN", "DATE_FORMAT", "COALESCE"];
            if !safe_funcs.contains(&func_name.as_str()) {
                return Err(format!("Unsafe or unsupported function call: {}", func_name));
            }
        } else {
            // just an identifier
            Self::validate_identifier(field)?;
        }
        Ok(())
    }

    pub fn is_safe_order_by(order: &str) -> bool {
        // "personal.id DESC" -> ["personal.id", "DESC"]
        let parts: Vec<&str> = order.split_whitespace().collect();
        if parts.is_empty() || parts.len() > 2 {
            return false;
        }
        
        if Self::validate_identifier(parts[0]).is_err() {
            return false;
        }
        
        if parts.len() == 2 {
            let dir = parts[1].to_uppercase();
            if dir != "ASC" && dir != "DESC" {
                return false;
            }
        }
        
        true
    }
}
