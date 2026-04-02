use std::collections::HashMap;
use serde_json::{json, Value};
use crate::models::{FilterRule, ParseResult, QueryNode, RootQuery};
use crate::guard::Guard;

pub struct SqlGenerator {
    param_counter: usize,
    params: HashMap<String, Value>,
    selects: Vec<String>,
    froms: Vec<String>,
    joins: Vec<String>,
    wheres: Vec<String>,
}

impl SqlGenerator {
    pub fn new() -> Self {
        Self {
            param_counter: 0,
            params: HashMap::new(),
            selects: Vec::new(),
            froms: Vec::new(),
            joins: Vec::new(),
            wheres: Vec::new(),
        }
    }

    pub fn generate(mut self, root: RootQuery) -> Result<ParseResult, String> {
        let mut root_structure = serde_json::Map::new();
        
        for node in root.nodes {
            let mut node_structure = serde_json::Map::new();
            self.process_node(&node, &mut node_structure)?;
            root_structure.insert(node.name.clone(), Value::Object(node_structure));
        }
        
        let mut sql = String::new();
        
        // SELECT
        sql.push_str("SELECT ");
        if self.selects.is_empty() {
            sql.push_str("*");
        } else {
            sql.push_str(&self.selects.join(", "));
        }
        
        // FROM
        if !self.froms.is_empty() {
            sql.push_str("\nFROM ");
            sql.push_str(&self.froms.join(", "));
        }
        
        // JOIN
        if !self.joins.is_empty() {
            sql.push_str("\n");
            sql.push_str(&self.joins.join("\n"));
        }
        
        // WHERE
        if !self.wheres.is_empty() {
            sql.push_str("\nWHERE ");
            sql.push_str(&self.wheres.join(" AND "));
        }
        
        // ORDER BY
        if let Some(order) = root.config.order {
            if Guard::is_safe_order_by(&order) {
                sql.push_str("\nORDER BY ");
                sql.push_str(&order);
            }
        }
        
        // LIMIT & OFFSET
        if let Some(limit) = root.config.limit {
            sql.push_str(&format!("\nLIMIT {}", limit));
        }
        if let Some(offset) = root.config.offset {
            sql.push_str(&format!("\nOFFSET {}", offset));
        }
        
        Ok(ParseResult {
            sql,
            params: self.params,
            structure: Value::Object(root_structure),
        })
    }

    fn process_node(&mut self, node: &QueryNode, structure: &mut serde_json::Map<String, Value>) -> Result<(), String> {
        // Table Name Handling
        let table_alias = &node.name;
        
        if let Some(source) = &node.source {
            Guard::validate_table(&source.table_name)?;
            
            // If it's the root node (has @source but no @join), it goes to FROM
            if node.join.is_none() {
                self.froms.push(format!("{} AS {}", source.table_name, table_alias));
            }
            
            // Process Filters
            for filter in &source.filters {
                let condition = self.build_condition(table_alias, filter)?;
                self.wheres.push(condition);
            }
        }
        
        // Joins
        if let Some(join) = &node.join {
            // Very simple validation check:
            if !join.to_uppercase().contains("JOIN") {
                return Err("Invalid JOIN syntax".to_string());
            }
            self.joins.push(join.clone());
        }
        
        // Fields
        let mut field_map = serde_json::Map::new();
        for (field_key, field_sql) in &node.fields {
            Guard::validate_field(field_sql)?;
            
            let alias = format!("{}_{}", table_alias, field_key);
            
            // E.g., CONCAT(...) or just a column name
            let processed_field = if field_sql.contains('(') {
                // Keep raw if it looks like a function, but we already validated it
                format!("{} AS {}", field_sql, alias)
            } else {
                // Prefix with table alias if no prefix exists
                if field_sql.contains('.') {
                    format!("{} AS {}", field_sql, alias)
                } else {
                    format!("{}.{} AS {}", table_alias, field_sql, alias)
                }
            };
            
            self.selects.push(processed_field);
            field_map.insert(field_key.to_string(), json!(alias));
        }
        
        // Merge fields to structure
        for (k, v) in field_map {
            structure.insert(k, v);
        }
        
        // Process children recursively
        for child in &node.children {
            let mut child_struct = serde_json::Map::new();
            self.process_node(child, &mut child_struct)?;
            structure.insert(child.name.clone(), Value::Object(child_struct));
        }
        
        Ok(())
    }

    fn build_condition(&mut self, table_alias: &str, filter: &FilterRule) -> Result<String, String> {
        Guard::validate_identifier(&filter.field)?;
        
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
                // Might be a number instead of string, we can try to parse it
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
                // handle "in (1, 2, 3)" or just "1, 2, 3"
                let val = filter.value.trim_matches(|c| c == '(' || c == ')');
                let parts: Vec<&str> = val.split(',').collect();
                let mut param_names = Vec::new();
                for part in parts {
                    param_names.push(self.next_param_numeric_or_string(part.trim()));
                }
                Ok(format!("{} IN ({})", column_ref, param_names.join(", ")))
            }
            "between" => {
                // "25..45"
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
        // Simple heuristic: if it parses as number, treat as number param, else string
        if let Ok(n) = val.parse::<f64>() {
            self.next_param(json!(n))
        } else {
            // Strip single quotes if they exist
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
