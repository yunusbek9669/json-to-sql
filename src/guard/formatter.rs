use super::{Guard, WhitelistRule};

impl Guard {
    pub fn expand_mapped_fields(&self, field_sql: &str, context: &str) -> String {
        let wl = match &self.whitelist {
            Some(w) => w,
            None => return field_sql.to_string(),
        };
        let rule = match wl.get(context) {
            Some(r) => r,
            None => return field_sql.to_string(),
        };

        if matches!(rule, WhitelistRule::Allowed(_)) {
            return field_sql.to_string(); // No expressions mapping to do
        }

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
                    let raw_field = if let Some((_, col)) = current_word.split_once('.') { col } else { &current_word };
                    let mapped = rule.get_mapping(raw_field).unwrap_or(current_word.clone());
                    result.push_str(&mapped);
                    current_word.clear();
                }
                result.push(c);
            } else if !in_quotes && (c.is_alphanumeric() || c == '_') {
                current_word.push(c);
            } else {
                if !current_word.is_empty() {
                    let raw_field = if let Some((_, col)) = current_word.split_once('.') { col } else { &current_word };
                    let is_numeric = current_word.parse::<f64>().is_ok();
                    if !is_numeric {
                        if let Some(mapped) = rule.get_mapping(raw_field) {
                            result.push_str(&mapped);
                        } else {
                            result.push_str(&current_word);
                        }
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
            let raw_field = if let Some((_, col)) = current_word.split_once('.') { col } else { &current_word };
            let is_numeric = current_word.parse::<f64>().is_ok();
            if !is_numeric {
                 if let Some(mapped) = rule.get_mapping(raw_field) {
                     result.push_str(&mapped);
                 } else {
                     result.push_str(&current_word);
                 }
            } else {
                 result.push_str(&current_word);
            }
        }
        
        result
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
