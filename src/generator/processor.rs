use crate::models::QueryNode;
use crate::guard::Guard;
use super::relation::extract_on_condition;
use super::SqlGenerator;

pub(crate) struct LateralResult {
    pub sql: String,
}

/// Escape single quotes for use inside SQL string literals: ' → ''
fn escape_sql_key(key: &str) -> String {
    key.replace('\'', "''")
}

impl SqlGenerator {
    pub(crate) fn process_node(&mut self, node: &QueryNode, context: Option<(&str, &str)>) -> Result<Vec<String>, String> {
        // source_name = current alias, real_table = real DB table
        // For structural nodes (no @source), we use the parent's context for JOINs and prefixes.
        let (current_alias, current_real) = if let Some(source) = &node.source {
            let real = self.guard.resolve_alias(&source.table_name)?;
            (source.table_name.clone(), real)
        } else if let Some((p_alias, p_real)) = context {
            (p_alias.to_string(), p_real.to_string())
        } else {
            (node.name.clone(), node.name.clone())
        };

        if let Some(source) = &node.source {
            self.guard.validate_table(&source.table_name)?;
            
            if context.is_none() {
                // Root node -> FROM clause
                self.froms.push(format!("{} AS {}", current_real, current_alias));
                self.joined_aliases.insert(current_alias.clone());
            } else if !node.is_list {
                // Non-list child -> JOIN clause
                if !self.joined_aliases.contains(&current_alias) {
                    let (p_alias, p_real) = context.unwrap();

                    // Resolve relationship hint
                    let is_numeric = node.name.chars().all(|c| c.is_numeric());
                    let rel_hint = if is_numeric { &source.table_name } else { &node.name };
                    let rel_name = source.rel.as_deref().unwrap_or(rel_hint);

                    let j_str = if let Some(j) = &node.join {
                        Some(j.clone())
                    } else {
                        self.resolve_relation(p_alias, &current_alias, p_real, &current_real, rel_name)?
                    };

                    if let Some(mut j) = j_str {
                        if !j.to_uppercase().contains("JOIN") {
                            return Err(format!("Invalid JOIN syntax for {}->{}", p_alias, &current_alias));
                        }
                        if let Some(jt) = &source.join_type {
                            j = Self::override_join_type(&j, jt);
                        }
                        self.joins.push(j);
                        self.joined_aliases.insert(current_alias.clone());
                    } else {
                        // BFS for auto-path
                        if let Some(path) = self.find_relation_path(p_alias, &current_alias) {
                            self.auto_join_path(&path)?;
                        } else {
                            return Err(format!("No connection found for {}->{}", p_alias, &current_alias));
                        }
                    }
                }
            }
            
            if !node.is_list {
                for filter in &source.filters {
                    let condition = self.build_condition(&current_alias, filter)?;
                    self.wheres.push(condition);
                }
            }
        }
        
        let mut json_args = Vec::new();
        
        // Add fields belonging to this node's alias
        for (field_key, field_sql) in &node.fields {
            self.guard.validate_field(&current_alias, field_sql)?;
            let expanded = self.guard.expand_mapped_fields(field_sql, &current_alias);
            let processed = Guard::auto_prefix_field(&expanded, &current_alias);
            json_args.push(format!("'{}', {}", escape_sql_key(field_key), processed));
        }
        
        // Process children
        for child in &node.children {
            if child.is_list {
                let list_result = self.build_lateral_subquery(child, &current_alias, &current_real)?;
                let lateral_alias = format!("{}_list", child.name);
                self.joins.push(format!("LEFT JOIN LATERAL (\n  {}\n) {} ON true", list_result.sql, lateral_alias));
                json_args.push(format!("'{}', {}.array_data", escape_sql_key(&child.name), lateral_alias));
            } else {
                // Pass current context down for structural integrity
                let child_args = self.process_node(child, Some((&current_alias, &current_real)))?;
                
                if child.flatten {
                    // Merge child's fields/sub-objects into this node's JSON
                    json_args.extend(child_args);
                } else {
                    // Wrap child into its own JSON object
                    // We must not lose values here: join child_args together
                    if !child_args.is_empty() {
                        json_args.push(format!("'{}', json_build_object({})", escape_sql_key(&child.name), child_args.join(", ")));
                    } else {
                        // Keep the structure even if empty
                        json_args.push(format!("'{}', '{{}}'::json", escape_sql_key(&child.name)));
                    }
                }
            }
        }
        
        Ok(json_args)
    }

    /// Builds a LATERAL subquery for list (One-to-Many) nodes.
    pub(crate) fn build_lateral_subquery(&mut self, node: &QueryNode, parent_alias: &str, parent_real: &str) -> Result<LateralResult, String> {
        let source = node.source.as_ref()
            .ok_or_else(|| format!("List node '{}' must have @source", node.name))?;
        
        let child_alias = &source.table_name;
        let real_table = self.guard.resolve_alias(&source.table_name)?;
        self.guard.validate_table(&source.table_name)?;
        
        // Build the inner json_build_object fields
        let mut inner_args = Vec::new();
        
        for (field_key, field_sql) in &node.fields {
            self.guard.validate_field(child_alias, field_sql)?;
            let expanded_field = self.guard.expand_mapped_fields(field_sql, child_alias);
            let processed = Guard::auto_prefix_field(&expanded_field, child_alias);
            inner_args.push(format!("'{}', {}", escape_sql_key(field_key), processed));
        }
        
        // Build inner joins and where for children of the list node
        // A fresh alias tracking set for the LATERAL scope
        // Start by prepopulating with the list node's alias to avoid re-joining itself
        let mut inner_aliases = std::collections::HashSet::new();
        inner_aliases.insert(child_alias.to_string());
        
        let old_joins = std::mem::take(&mut self.joins);
        let old_wheres = std::mem::take(&mut self.wheres);
        let old_froms = std::mem::take(&mut self.froms);
        let old_aliases = std::mem::replace(&mut self.joined_aliases, inner_aliases);
        
        // Iterate and process children recursively like a normal scoped tree
        for child in &node.children {
            let child_args = self.process_node(
                child,
                Some((child_alias, &real_table))
            )?;
            
            if child.flatten {
                inner_args.extend(child_args);
            } else {
                inner_args.push(format!("'{}', json_build_object({})", escape_sql_key(&child.name), child_args.join(", ")));
            }
        }
        
        // Retrieve the scoped joins and wheres
        let inner_joins = std::mem::replace(&mut self.joins, old_joins);
        let inner_wheres = std::mem::replace(&mut self.wheres, old_wheres);
        self.froms = old_froms;
        self.joined_aliases = old_aliases;
        
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
            let cond = self.build_condition(child_alias, filter)?;
            where_parts.push(cond);
        }
        where_parts.extend(inner_wheres);
        
        // Build inner SELECT (rows with ORDER BY / LIMIT)
        let mut inner_sql = format!(
            "SELECT {} AS item\n    FROM {} AS {}",
            json_obj, real_table, child_alias
        );
        
        // Add inner joins
        for ij in &inner_joins {
            inner_sql.push_str(&format!("\n    {}", ij));
        }
        
        inner_sql.push_str(&format!("\n    WHERE {}", where_parts.join(" AND ")));
        
        if let Some(order) = &source.order {
            if self.guard.is_safe_order_by(order).is_ok() {
                let prefixed_order = Guard::auto_prefix_field(order, child_alias);
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
        
        Ok(LateralResult { sql })
    }
}
