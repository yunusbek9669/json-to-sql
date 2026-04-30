use regex::Regex;
use super::Guard;

impl Guard {
    pub fn validate_table(&self, context: &str) -> Result<(), String> {
        Self::check_global_threats(context)?;

        static RE_TABLE: once_cell::sync::Lazy<Regex> =
            once_cell::sync::Lazy::new(|| Regex::new(r"^[a-zA-Z0-9_]+$").unwrap());
        if !RE_TABLE.is_match(context) {
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

        static RE_COLUMN: once_cell::sync::Lazy<Regex> =
            once_cell::sync::Lazy::new(|| Regex::new(r"^[a-zA-Z0-9_\.]+$").unwrap());
        if !RE_COLUMN.is_match(field) {
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
                    static RE_STR:   once_cell::sync::Lazy<Regex> =
                        once_cell::sync::Lazy::new(|| Regex::new(r"'[^']*'").unwrap());
                    static RE_IDENT: once_cell::sync::Lazy<Regex> =
                        once_cell::sync::Lazy::new(|| Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]*").unwrap());
                    let field_no_str = RE_STR.replace_all(field, "");
                    let re_ident = &*RE_IDENT;

                    for m in re_ident.find_iter(field_no_str.as_ref()) {
                        let ident = m.as_str();
                        if builtins.contains(&ident.to_uppercase().as_str()) { continue; }
                        if ident.parse::<f64>().is_ok() { continue; }

                        // FIX #10: when an identifier matches a local (flattened) alias,
                        // the identifier will be *substituted* with the alias SQL value by
                        // auto_prefix_field — the whitelist check for the current table is
                        // intentionally skipped.  We add a defense-in-depth re-validation of
                        // the alias VALUE itself so that a dangerous expression can never
                        // reach the final SQL even if child-processing somehow missed it.
                        if let Some(aliases) = local_aliases {
                            if let Some(alias_sql) = aliases.get(ident) {
                                Guard::check_global_threats(alias_sql)?;
                                continue;
                            }
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

        // FIX #1: detect SELECT in any whitespace form — "SELECT(", "(SELECT", "SELECT\t", etc.
        // Strip all whitespace then check for the bare word to catch "SELECT(1)" style injections.
        let compact_upper = field_upper.split_whitespace().collect::<Vec<_>>().join(" ");
        let no_space_upper = field_upper.replace(char::is_whitespace, "");

        // 1. Allow CASE expressions directly
        if field_upper.starts_with("CASE ") && field_upper.ends_with(" END") {
            if no_space_upper.contains("SELECT") {
                return Err("Unsafe CASE expression: SELECT is not allowed".to_string());
            }
            return Ok(());
        }

        // 2. Allow Function Calls
        if field.contains('(') {
            let func_name = field.split('(').next().unwrap_or("").trim().to_uppercase();
            if !builtins.contains(&func_name.as_str()) {
                return Err(format!("Unsafe or unsupported function call: {}", func_name));
            }
            // FIX #1: check SELECT in compact form too ("SELECT(", "(SELECT", etc.)
            if compact_upper.contains("SELECT ") || no_space_upper.contains("SELECT(") || no_space_upper.contains("(SELECT") {
                return Err("Subqueries are not allowed in field expressions".to_string());
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

        static RE_ORDER: once_cell::sync::Lazy<Regex> =
            once_cell::sync::Lazy::new(|| Regex::new(r"^[a-zA-Z0-9_\.]+$").unwrap());
        if !RE_ORDER.is_match(parts[0]) {
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
