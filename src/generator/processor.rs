use crate::models::QueryNode;
use crate::guard::Guard;
use super::relation::extract_on_condition;
use super::SqlGenerator;

pub(crate) struct LateralResult {
    pub sql: String,
}

impl SqlGenerator {
    pub(crate) fn process_node(&mut self, node: &QueryNode, parent_table: Option<(&str, &str)>) -> Result<Vec<String>, String> {
        // source_name = original name from frontend (alias), real_table = resolved real DB name
        let (source_name, real_table) = if let Some(source) = &node.source {
            let real = self.guard.resolve_alias(&source.table_name)?;
            (source.table_name.clone(), real)
        } else {
            (node.name.clone(), node.name.clone())
        };
        
        if let Some(source) = &node.source {
            self.guard.validate_table(&source.table_name)?;
            
            // If parent_table is None, this is a root node -> FROM
            if parent_table.is_none() {
                self.froms.push(format!("{} AS {}", real_table, source_name));
                self.joined_aliases.insert(source_name.clone());
            } else if !node.is_list {
                // Normal scalar child -> regular JOIN
                // Skip if already joined (e.g., by auto-path resolution)
                if !self.joined_aliases.contains(&source_name) {
                    let (p_alias, p_real) = parent_table.unwrap();

                    // $rel modifier: use explicit relation name for lookup
                    let rel_name = source.rel.as_deref().unwrap_or(&node.name);

                    let j_str = if let Some(j) = &node.join {
                        Some(j.clone())
                    } else {
                        self.resolve_relation(p_alias, &source_name, p_real, &real_table, rel_name)?
                    };

                    if let Some(mut j) = j_str {
                        if !j.to_uppercase().contains("JOIN") {
                            return Err(format!("Invalid JOIN syntax for {}->{}", p_alias, &source_name));
                        }
                        // $join modifier: override JOIN type (e.g., "left" → "LEFT JOIN")
                        if let Some(jt) = &source.join_type {
                            j = Self::override_join_type(&j, jt);
                        }
                        self.joins.push(j);
                        self.joined_aliases.insert(source_name.clone());
                    } else {
                        // No direct relation — try auto-path resolution via BFS
                        if let Some(path) = self.find_relation_path(p_alias, &source_name) {
                            self.auto_join_path(&path)?;
                        } else {
                            return Err(format!("No @join provided and no relation path found for {}->{}", p_alias, &source_name));
                        }
                    }
                }
            }
            // is_list nodes are handled separately below (LATERAL subquery)
            
            // Process Filters (only for non-list or root nodes)
            if !node.is_list {
                for filter in &source.filters {
                    let condition = self.build_condition(&source_name, filter)?;
                    self.wheres.push(condition);
                }
            }
        }
        
        // Construct the parts inside JSON_OBJECT for this node
        let mut json_object_args = Vec::new();
        
        // Fields
        for (field_key, field_sql) in &node.fields {
            self.guard.validate_field(&source_name, field_sql)?;
            let expanded_field = self.guard.expand_mapped_fields(field_sql, &source_name);
            let processed_field = Guard::auto_prefix_field(&expanded_field, &source_name);
            json_object_args.push(format!("'{}', {}", field_key, processed_field));
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
            } else {
                let child_args = self.process_node(child, Some((&source_name, &real_table)))?;
                
                if child.flatten {
                    json_object_args.extend(child_args);
                } else {
                    json_object_args.push(format!("'{}', json_build_object({})", child.name, child_args.join(", ")));
                }
            }
        }
        
        Ok(json_object_args)
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
            inner_args.push(format!("'{}', {}", field_key, processed));
        }
        
        // Build inner joins and where for children of the list node
        let mut inner_joins = Vec::new();
        let mut inner_wheres = Vec::new();
        
        // Recursively collect flatten children fields
        self.collect_list_children(node, child_alias, &real_table, &mut inner_args, &mut inner_joins, &mut inner_wheres)?;
        
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
    
    /// Recursively collect flatten children of a list node
    pub(crate) fn collect_list_children(
        &mut self,
        node: &QueryNode,
        _parent_alias: &str,
        _parent_real: &str,
        args: &mut Vec<String>,
        joins: &mut Vec<String>,
        wheres: &mut Vec<String>,
    ) -> Result<(), String> {
        for child in &node.children {
            let child_source = child.source.as_ref();
            
            if let Some(source) = child_source {
                let child_alias_name = &source.table_name;
                let child_real = self.guard.resolve_alias(&source.table_name)?;
                self.guard.validate_table(child_alias_name)?;
                
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
                    let cond = self.build_condition(child_alias_name, filter)?;
                    wheres.push(cond);
                }
                
                for (fk, fv) in &child.fields {
                    self.guard.validate_field(child_alias_name, fv)?;
                    let expanded_field = self.guard.expand_mapped_fields(fv, child_alias_name);
                    let processed = Guard::auto_prefix_field(&expanded_field, child_alias_name);
                    args.push(format!("'{}', {}", fk, processed));
                }
                
                // Recurse for deeper children
                self.collect_list_children(child, child_alias_name, &child_real, args, joins, wheres)?;
            }
        }
        Ok(())
    }
}
