use serde_json::{json, Value};
use crate::models::FilterRule;
use crate::guard::Guard;
use super::SqlGenerator;

impl SqlGenerator {
    pub(crate) fn build_condition(&mut self, table_alias: &str, filter: &FilterRule, node_fields: Option<&indexmap::IndexMap<String, String>>) -> Result<String, String> {
        let is_macro_field = node_fields.map_or(false, |f| f.contains_key(&filter.field));
        
        let expanded_field = if is_macro_field {
            let expr = node_fields.unwrap().get(&filter.field).unwrap();
            self.guard.validate_field(table_alias, expr, None)?;
            self.guard.expand_mapped_fields(expr, table_alias)
        } else {
            self.guard.validate_column(table_alias, &filter.field)?;
            self.guard.expand_mapped_fields(&filter.field, table_alias)
        };
        
        let column_ref = Guard::auto_prefix_field(&expanded_field, table_alias, None);
        
        match filter.operator.as_str() {
            "eq" => {
                let p = self.next_param(Value::String(filter.value.clone()));
                Ok(format!("{} = {}", column_ref, p))
            }
            "neq" => {
                let p = self.next_param(Value::String(filter.value.clone()));
                Ok(format!("{} != {}", column_ref, p))
            }
            "gt" => {
                let p = self.next_param_numeric_or_string(&filter.value);
                Ok(format!("{} > {}", column_ref, p))
            }
            "lt" => {
                let p = self.next_param_numeric_or_string(&filter.value);
                Ok(format!("{} < {}", column_ref, p))
            }
            "like" => {
                let p = self.next_param(Value::String(filter.value.clone()));
                Ok(format!("{} LIKE {}", column_ref, p))
            }
            "in" => {
                let val = filter.value.trim_matches(|c| c == '(' || c == ')');
                let parts: Vec<&str> = val.split(',').collect();
                let mut param_names = Vec::new();
                for part in parts {
                    param_names.push(self.next_param_numeric_or_string(part.trim()));
                }
                Ok(format!("{} IN ({})", column_ref, param_names.join(", ")))
            }
            "between" => {
                if let Some((start, end)) = filter.value.split_once("..") {
                    let p1 = self.next_param_numeric_or_string(start.trim());
                    let p2 = self.next_param_numeric_or_string(end.trim());
                    Ok(format!("{} BETWEEN {} AND {}", column_ref, p1, p2))
                } else {
                    Err(format!("Invalid between syntax: {}", filter.value))
                }
            }
            _ => Err(format!("Unsupported operator: {}", filter.operator))
        }
    }

    pub(crate) fn next_param_numeric_or_string(&mut self, val: &str) -> String {
        if let Ok(n) = val.parse::<f64>() {
            self.next_param(json!(n))
        } else {
            // FIX #7: trim_matches('\'') strips ALL leading/trailing quotes, not just one pair.
            // Use explicit pair removal so "'test'" → "test" but "'''evil'''" → "''evil''" stays.
            let clean_val = if val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2 {
                val[1..val.len() - 1].to_string()
            } else {
                val.to_string()
            };
            self.next_param(Value::String(clean_val))
        }
    }

    pub(crate) fn next_param(&mut self, val: Value) -> String {
        self.param_counter += 1;
        let param_name = format!("p{}", self.param_counter);
        let param_placeholder = format!(":{}", param_name);
        self.params.insert(param_name, val);
        param_placeholder
    }
}
