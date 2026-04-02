use std::collections::HashMap;
use serde_json::{json, Value};
use crate::models::{FilterRule, ParseResult, QueryNode, RootQuery};
use crate::guard::Guard;

use indexmap::IndexMap;

pub struct SqlGenerator {
    param_counter: usize,
    params: IndexMap<String, Value>,
    froms: Vec<String>,
    joins: Vec<String>,
    wheres: Vec<String>,
    guard: Guard,
    relations: HashMap<String, String>,
}

impl SqlGenerator {
    pub fn new(whitelist: Option<HashMap<String, Vec<String>>>, relations: Option<HashMap<String, String>>) -> Self {
        Self {
            param_counter: 0,
            params: IndexMap::new(),
            froms: Vec::new(),
            joins: Vec::new(),
            wheres: Vec::new(),
            guard: Guard::new(whitelist),
            relations: relations.unwrap_or_default(),
        }
    }

    pub fn generate(mut self, root: RootQuery) -> Result<ParseResult, String> {
        let mut root_structure = serde_json::Map::new();
        let mut root_json_objects = Vec::new();
        
        for node in root.nodes {
            let mut node_structure = serde_json::Map::new();
            let child_args = self.process_node(&node, None, &mut node_structure)?;
            let json_obj_expr = format!("json_build_object({})", child_args.join(", "));
            
            root_structure.insert(node.name.clone(), Value::Object(node_structure));
            // Just push the generated object, dropping the outer name wrapper
            root_json_objects.push(json_obj_expr);
        }
        
        // BASE SQL Construction
        let mut base_sql = String::new();
        base_sql.push_str("SELECT ");
        
        if root_json_objects.is_empty() {
            base_sql.push_str("*");
        } else if root_json_objects.len() == 1 {
            base_sql.push_str(&format!("{} AS uaq_data", root_json_objects[0]));
        } else {
            // Fallback for multiple roots in the same query
            let mut fallback_args = Vec::new();
            for (i, expr) in root_json_objects.iter().enumerate() {
                fallback_args.push(format!("'root{}', {}", i, expr));
            }
            base_sql.push_str(&format!("json_build_object({}) AS uaq_data", fallback_args.join(", ")));
        }
        
        if !self.froms.is_empty() {
            base_sql.push_str("\nFROM ");
            base_sql.push_str(&self.froms.join(", "));
        }
        
        if !self.joins.is_empty() {
            base_sql.push_str("\n");
            base_sql.push_str(&self.joins.join("\n"));
        }
        
        if !self.wheres.is_empty() {
            base_sql.push_str("\nWHERE ");
            base_sql.push_str(&self.wheres.join(" AND "));
        }
        
        if let Some(order) = root.config.order {
            if self.guard.is_safe_order_by(&order).is_ok() {
                base_sql.push_str("\nORDER BY ");
                base_sql.push_str(&order);
            }
        }
        
        if let Some(limit) = root.config.limit {
            base_sql.push_str(&format!("\nLIMIT {}", limit));
        }
        if let Some(offset) = root.config.offset {
            base_sql.push_str(&format!("\nOFFSET {}", offset));
        }
        
        // Wrap everything in json_agg to return a single JSON Array string
        let mut final_sql = String::new();
        if root_json_objects.is_empty() {
            final_sql = base_sql;
        } else {
            final_sql.push_str("SELECT COALESCE(json_agg(t.uaq_data), '[]'::json) \nFROM (\n");
            // Indent the base sql visually inside the subquery
            for line in base_sql.lines() {
                final_sql.push_str("  ");
                final_sql.push_str(line);
                final_sql.push_str("\n");
            }
            final_sql.push_str(") t");
        }
        
        Ok(ParseResult {
            is_ok: true,
            sql: Some(final_sql),
            params: Some(self.params),
            structure: Some(Value::Object(root_structure)),
            message: "success".to_string(),
        })
    }

    fn process_node(&mut self, node: &QueryNode, parent_table: Option<&str>, structure: &mut serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
        let table_alias = if let Some(source) = &node.source {
            source.table_name.clone()
        } else {
            node.name.clone()
        };
        
        if let Some(source) = &node.source {
            self.guard.validate_table(&source.table_name)?;
            
            // If parent_table is None, this is a root node -> FROM
            if parent_table.is_none() {
                self.froms.push(format!("{} AS {}", source.table_name, table_alias));
            } else {
                let j_str = if let Some(j) = &node.join {
                    Some(j.clone())
                } else {
                    let key = format!("{}->{}", parent_table.unwrap(), source.table_name);
                    self.relations.get(&key).cloned()
                };

                if let Some(j) = j_str {
                    if !j.to_uppercase().contains("JOIN") {
                        return Err(format!("Invalid JOIN syntax inside relation map for {}->{}", parent_table.unwrap(), source.table_name));
                    }
                    self.joins.push(j);
                } else {
                    return Err(format!("No @join provided and no relation defined for {}->{}", parent_table.unwrap(), source.table_name));
                }
            }
            
            // Process Filters
            for filter in &source.filters {
                let condition = self.build_condition(&table_alias, filter)?;
                self.wheres.push(condition);
            }
        }
        
        // Construct the parts inside JSON_OBJECT for this node
        let mut json_object_args = Vec::new();
        
        // Fields
        for (field_key, field_sql) in &node.fields {
            self.guard.validate_field(&table_alias, field_sql)?;
            
            let processed_field = if field_sql.contains('(') {
                field_sql.clone()
            } else if field_sql.to_uppercase().trim().starts_with("CASE ") {
                field_sql.clone()
            } else if field_sql.starts_with('\'') && field_sql.ends_with('\'') {
                field_sql.clone()
            } else if field_sql.parse::<f64>().is_ok() {
                field_sql.clone()
            } else if field_sql.contains('.') {
                field_sql.clone()
            } else {
                format!("{}.{}", table_alias, field_sql)
            };
            
            json_object_args.push(format!("'{}', {}", field_key, processed_field));
            
            // Still populate structure for backwards compatibility / metadata if needed
            structure.insert(field_key.to_string(), json!(processed_field));
        }
        
        // Process children recursively
        for child in &node.children {
            let mut child_struct = serde_json::Map::new();
            let child_args = self.process_node(child, Some(&table_alias), &mut child_struct)?;
            
            if child.flatten {
                // Flatten: add the child arguments to the current level without wrapper
                json_object_args.extend(child_args);
                for (k, v) in child_struct {
                    structure.insert(k, v);
                }
            } else {
                json_object_args.push(format!("'{}', json_build_object({})", child.name, child_args.join(", ")));
                structure.insert(child.name.clone(), Value::Object(child_struct));
            }
        }
        
        Ok(json_object_args)
    }

    fn build_condition(&mut self, table_alias: &str, filter: &FilterRule) -> Result<String, String> {
        self.guard.validate_column(table_alias, &filter.field)?;
        
        let column_ref = format!("{}.{}", table_alias, filter.field);
        
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

    fn next_param_numeric_or_string(&mut self, val: &str) -> String {
        if let Ok(n) = val.parse::<f64>() {
            self.next_param(json!(n))
        } else {
            let clean_val = val.trim_matches('\'').to_string();
            self.next_param(Value::String(clean_val))
        }
    }

    fn next_param(&mut self, val: Value) -> String {
        self.param_counter += 1;
        let param_name = format!("p{}", self.param_counter);
        let param_placeholder = format!(":{}", param_name);
        self.params.insert(param_name, val);
        param_placeholder
    }
}
