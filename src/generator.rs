use std::collections::HashMap;
use serde_json::{json, Value};
use crate::models::{FilterRule, ParseResult, QueryNode};
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

    pub fn generate(mut self, root: QueryNode) -> Result<ParseResult, String> {
        let mut root_structure = serde_json::Map::new();
        
        let child_args = self.process_node(&root, None, &mut root_structure)?;
        let json_obj_expr = format!("json_build_object({})", child_args.join(", "));
        
        // BASE SQL Construction
        let mut base_sql = String::new();
        base_sql.push_str("SELECT ");
        base_sql.push_str(&format!("{} AS uaq_data", json_obj_expr));
        
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
        
        // Order, limit, offset from root node's @source
        if let Some(source) = &root.source {
            let root_table = self.guard.resolve_alias(&source.table_name)?;
            if let Some(order) = &source.order {
                if self.guard.is_safe_order_by(order).is_ok() {
                    let prefixed_order = Guard::auto_prefix_field(order, &root_table);
                    base_sql.push_str("\nORDER BY ");
                    base_sql.push_str(&prefixed_order);
                }
            }
            if let Some(limit) = source.limit {
                base_sql.push_str(&format!("\nLIMIT {}", limit));
            }
            if let Some(offset) = source.offset {
                base_sql.push_str(&format!("\nOFFSET {}", offset));
            }
        }
        
        // Wrap in json_agg
        let mut final_sql = String::new();
        final_sql.push_str("SELECT COALESCE(json_agg(t.uaq_data), '[]'::json) \nFROM (\n");
        for line in base_sql.lines() {
            final_sql.push_str("  ");
            final_sql.push_str(line);
            final_sql.push_str("\n");
        }
        final_sql.push_str(") t");
        
        Ok(ParseResult {
            is_ok: true,
            sql: Some(final_sql),
            params: Some(self.params),
            structure: Some(Value::Object(root_structure)),
            message: "success".to_string(),
        })
    }

    fn process_node(&mut self, node: &QueryNode, parent_table: Option<(&str, &str)>, structure: &mut serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
        // source_name = original name from frontend (alias), real_table = resolved real DB name
        let (source_name, real_table) = if let Some(source) = &node.source {
            let real = self.guard.resolve_alias(&source.table_name)?;
            (source.table_name.clone(), real)
        } else {
            (node.name.clone(), node.name.clone())
        };
        
        if let Some(source) = &node.source {
            self.guard.validate_table(&real_table)?;
            
            // If parent_table is None, this is a root node -> FROM
            if parent_table.is_none() {
                self.froms.push(format!("{} AS {}", real_table, real_table));
            } else if !node.is_list {
                // Normal scalar child -> regular JOIN
                let (p_alias, p_real) = parent_table.unwrap();

                let j_str = if let Some(j) = &node.join {
                    Some(j.clone())
                } else {
                    self.resolve_relation(p_alias, &source_name, p_real, &real_table, &node.name)?
                };

                if let Some(j) = j_str {
                    if !j.to_uppercase().contains("JOIN") {
                        return Err(format!("Invalid JOIN syntax for {}->{}", p_alias, &source_name));
                    }
                    self.joins.push(j);
                } else {
                    return Err(format!("No @join provided and no relation defined for {}->{}", p_alias, &source_name));
                }
            }
            // is_list nodes are handled separately below (LATERAL subquery)
            
            // Process Filters (only for non-list or root nodes)
            if !node.is_list {
                for filter in &source.filters {
                    let condition = self.build_condition(&real_table, filter)?;
                    self.wheres.push(condition);
                }
            }
        }
        
        // Construct the parts inside JSON_OBJECT for this node
        let mut json_object_args = Vec::new();
        
        // Fields
        for (field_key, field_sql) in &node.fields {
            self.guard.validate_field(&real_table, field_sql)?;
            let processed_field = Guard::auto_prefix_field(field_sql, &real_table);
            json_object_args.push(format!("'{}', {}", field_key, processed_field));
            
            structure.insert(field_key.to_string(), json!(processed_field));
        }
        
        // Process children recursively
        for child in &node.children {
            if child.is_list {
                // --- LATERAL SUBQUERY for list (One-to-Many) ---
                let list_result = self.build_lateral_subquery(child, &source_name, &real_table)?;
                let lateral_alias = format!("{}_list", child.name);
                
                self.joins.push(format!(
                    "LEFT JOIN LATERAL (\n  {}\n) {} ON true",
                    list_result.sql, lateral_alias
                ));
                
                json_object_args.push(format!("'{}', {}.array_data", child.name, lateral_alias));
                
                let mut child_struct = serde_json::Map::new();
                child_struct.insert("_type".to_string(), json!("array"));
                for (k, v) in list_result.structure {
                    child_struct.insert(k, v);
                }
                structure.insert(child.name.clone(), Value::Object(child_struct));
            } else {
                let mut child_struct = serde_json::Map::new();
                let child_args = self.process_node(child, Some((&source_name, &real_table)), &mut child_struct)?;
                
                if child.flatten {
                    json_object_args.extend(child_args);
                    for (k, v) in child_struct {
                        structure.insert(k, v);
                    }
                } else {
                    json_object_args.push(format!("'{}', json_build_object({})", child.name, child_args.join(", ")));
                    structure.insert(child.name.clone(), Value::Object(child_struct));
                }
            }
        }
        
        Ok(json_object_args)
    }

    /// Builds a LATERAL subquery for list (One-to-Many) nodes.
    fn build_lateral_subquery(&mut self, node: &QueryNode, parent_alias: &str, parent_real: &str) -> Result<LateralResult, String> {
        let source = node.source.as_ref()
            .ok_or_else(|| format!("List node '{}' must have @source", node.name))?;
        
        let child_alias = &source.table_name;
        let real_table = self.guard.resolve_alias(&source.table_name)?;
        self.guard.validate_table(&real_table)?;
        
        // Build the inner json_build_object fields
        let mut inner_args = Vec::new();
        let mut inner_structure = serde_json::Map::new();
        
        for (field_key, field_sql) in &node.fields {
            self.guard.validate_field(&real_table, field_sql)?;
            let processed = Guard::auto_prefix_field(field_sql, &real_table);
            inner_args.push(format!("'{}', {}", field_key, processed));
            inner_structure.insert(field_key.to_string(), json!(processed));
        }
        
        // Build inner joins and where for children of the list node
        let mut inner_joins = Vec::new();
        let mut inner_wheres = Vec::new();
        
        // Recursively collect flatten children fields
        self.collect_list_children(node, child_alias, &real_table, &mut inner_args, &mut inner_joins, &mut inner_wheres, &mut inner_structure)?;
        
        let json_obj = format!("json_build_object({})", inner_args.join(", "));
        
        // Build the JOIN between parent and this list node
        let join_condition = if let Some(j) = &node.join {
            extract_on_condition(j)?
        } else {
            let resolved = self.resolve_relation(parent_alias, child_alias, parent_real, &real_table, &node.name)?;
            match resolved {
                Some(r) => extract_on_condition(&r)?,
                None => return Err(format!("No relation defined for list {}->{}", parent_alias, child_alias)),
            }
        };
        
        // Build WHERE clause
        let mut where_parts = vec![join_condition];
        for filter in &source.filters {
            let cond = self.build_condition(&real_table, filter)?;
            where_parts.push(cond);
        }
        where_parts.extend(inner_wheres);
        
        // Build inner SELECT (rows with ORDER BY / LIMIT)
        let mut inner_sql = format!(
            "SELECT {} AS item\n    FROM {}",
            json_obj, real_table
        );
        
        // Add inner joins
        for ij in &inner_joins {
            inner_sql.push_str(&format!("\n    {}", ij));
        }
        
        inner_sql.push_str(&format!("\n    WHERE {}", where_parts.join(" AND ")));
        
        if let Some(order) = &source.order {
            if self.guard.is_safe_order_by(order).is_ok() {
                let prefixed_order = Guard::auto_prefix_field(order, &real_table);
                inner_sql.push_str(&format!("\n    ORDER BY {}", prefixed_order));
            }
        }
        if let Some(limit) = source.limit {
            inner_sql.push_str(&format!("\n    LIMIT {}", limit));
        }
        if let Some(offset) = source.offset {
            inner_sql.push_str(&format!("\n    OFFSET {}", offset));
        }
        
        // Wrap with json_agg on the outside
        let sql = format!(
            "SELECT COALESCE(json_agg(sub.item), '[]'::json) AS array_data\n  FROM (\n    {}\n  ) sub",
            inner_sql
        );
        
        Ok(LateralResult { sql, structure: inner_structure })
    }
    
    /// Recursively collect flatten children of a list node
    fn collect_list_children(
        &mut self,
        node: &QueryNode,
        _parent_alias: &str,
        _parent_real: &str,
        args: &mut Vec<String>,
        joins: &mut Vec<String>,
        wheres: &mut Vec<String>,
        structure: &mut serde_json::Map<String, Value>,
    ) -> Result<(), String> {
        for child in &node.children {
            let child_source = child.source.as_ref();
            
            if let Some(source) = child_source {
                let child_alias_name = &source.table_name;
                let child_real = self.guard.resolve_alias(&source.table_name)?;
                self.guard.validate_table(&child_real)?;
                
                let (pa, pr) = if let Some(ns) = &node.source {
                    (ns.table_name.clone(), self.guard.resolve_alias(&ns.table_name)?)
                } else {
                    (_parent_alias.to_string(), _parent_real.to_string())
                };
                
                let j_str = if let Some(j) = &child.join {
                    Some(j.clone())
                } else {
                    self.resolve_relation(&pa, child_alias_name, &pr, &child_real, &child.name)?
                };
                
                if let Some(j) = j_str {
                    joins.push(j);
                } else {
                    return Err(format!("No relation for {}->{}", pa, child_alias_name));
                }
                
                for filter in &source.filters {
                    let cond = self.build_condition(&child_real, filter)?;
                    wheres.push(cond);
                }
                
                for (fk, fv) in &child.fields {
                    self.guard.validate_field(&child_real, fv)?;
                    let processed = Guard::auto_prefix_field(fv, &child_real);
                    
                    if child.flatten {
                        args.push(format!("'{}', {}", fk, processed));
                        structure.insert(fk.to_string(), json!(processed));
                    } else {
                        args.push(format!("'{}', {}", fk, processed));
                        structure.insert(fk.to_string(), json!(processed));
                    }
                }
                
                // Recurse for deeper children
                self.collect_list_children(child, child_alias_name, &child_real, args, joins, wheres, structure)?;
            }
        }
        Ok(())
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

    /// Resolves a relation template from the relations map.
    /// `parent_alias`/`child_alias` — used for KEY lookup (relation keys use aliases).
    /// `parent_real`/`child_real` — used for TEMPLATE replacement (@1, @2, @table → real names in SQL).
    /// Supports `->` (directional), `<->` (bi-directional) keys.
    /// Supports `:node_name` suffix for disambiguating same-table relations.
    fn resolve_relation(
        &self,
        parent_alias: &str, child_alias: &str,
        parent_real: &str, child_real: &str,
        node_name: &str,
    ) -> Result<Option<String>, String> {
        // Specific keys (with :node_name suffix)
        let spec_dir = format!("{}->{}:{}", parent_alias, child_alias, node_name);
        let spec_bi1 = format!("{}<->{}:{}", parent_alias, child_alias, node_name);
        let spec_bi2 = format!("{}<->{}:{}", child_alias, parent_alias, node_name);
        // Generic keys
        let key_dir = format!("{}->{}", parent_alias, child_alias);
        let key_bi1 = format!("{}<->{}", parent_alias, child_alias);
        let key_bi2 = format!("{}<->{}", child_alias, parent_alias);

        let candidates: Vec<(&str, bool)> = vec![
            (&spec_dir, false),
            (&spec_bi1, false),
            (&spec_bi2, true),
            (&key_dir, false),
            (&key_bi1, false),
            (&key_bi2, true),
        ];

        for (key, reversed) in candidates {
            if let Some(r) = self.relations.get(key) {
                // Validate: template must use @1, @2, @table — not raw alias names
                let (t1, t2) = if reversed { (child_alias, parent_alias) } else { (parent_alias, child_alias) };
                Self::validate_relation_template(r, t1, t2, key)?;
                // Replace with REAL table names for SQL
                let resolved = if reversed {
                    r.replace("@1", child_real)
                     .replace("@2", parent_real)
                     .replace("@table", child_real)
                } else {
                    r.replace("@1", parent_real)
                     .replace("@2", child_real)
                     .replace("@table", child_real)
                };
                return Ok(Some(resolved));
            }
        }

        Ok(None)
    }

    /// Checks that a relation template does NOT contain raw table names.
    /// Only @1, @2, @table placeholders are allowed.
    fn validate_relation_template(template: &str, table1: &str, table2: &str, key: &str) -> Result<(), String> {
        // Remove all valid placeholders first, then check for raw names
        let cleaned = template
            .replace("@table", "")
            .replace("@1", "")
            .replace("@2", "");
        
        // Check if any raw table name still appears (with dot after it = column reference)
        if cleaned.contains(&format!("{}.", table1)) {
            return Err(format!(
                "Relations Error [{}]: Raw table name '{}.' used directly. Use @1, @2, or @table placeholders instead.",
                key, table1
            ));
        }
        if cleaned.contains(&format!("{}.", table2)) {
            return Err(format!(
                "Relations Error [{}]: Raw table name '{}.' used directly. Use @1, @2, or @table placeholders instead.",
                key, table2
            ));
        }
        
        // Also check JOIN target (after JOIN keyword, before ON)
        let upper = template.to_uppercase();
        if let Some(join_pos) = upper.find("JOIN ") {
            let after_join = &template[join_pos + 5..];
            let target = after_join.split_whitespace().next().unwrap_or("");
            if target != "@table" && target != "@1" && target != "@2" {
                return Err(format!(
                    "Relations Error [{}]: Raw table name '{}' used after JOIN. Use @table, @1, or @2 placeholder instead.",
                    key, target
                ));
            }
        }
        
        Ok(())
    }
}

struct LateralResult {
    sql: String,
    structure: serde_json::Map<String, Value>,
}

/// Extracts the ON condition from a JOIN string.
/// E.g. "INNER JOIN foo ON bar.id = foo.bar_id AND foo.status = 1"
/// => "bar.id = foo.bar_id AND foo.status = 1"
fn extract_on_condition(join_str: &str) -> Result<String, String> {
    let upper = join_str.to_uppercase();
    if let Some(pos) = upper.find(" ON ") {
        Ok(join_str[pos + 4..].trim().to_string())
    } else {
        Err(format!("Cannot extract ON condition from: {}", join_str))
    }
}
