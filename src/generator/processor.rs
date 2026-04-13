use crate::models::QueryNode;
use crate::guard::Guard;
use super::relation::extract_on_condition;
use super::SqlGenerator;

pub(crate) struct LateralResult {
    pub sql: String,
}

fn escape_sql_key(key: &str) -> String {
    key.replace('\'', "''")
}

impl SqlGenerator {
    pub(crate) fn process_node(&mut self, node: &QueryNode, context: Option<(&str, &str)>) -> Result<Vec<String>, String> {
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
                self.froms.push(format!("{} AS {}", current_real, current_alias));
                self.joined_aliases.insert(current_alias.clone());
            } else if !node.is_list {
                if !self.joined_aliases.contains(&current_alias) {
                    let (p_alias, p_real) = context.unwrap();
                    let is_numeric = node.name.chars().all(|c| c.is_numeric());
                    let rel_hint = if is_numeric { &source.table_name } else { &node.name };
                    let rel_name = source.rel.as_deref().unwrap_or(rel_hint);
                    let j_str = if let Some(j) = &node.join { Some(j.clone()) } 
                                 else { self.resolve_relation(p_alias, &current_alias, p_real, &current_real, rel_name)? };

                    if let Some(mut j) = j_str {
                        if let Some(jt) = &source.join_type {
                            j = Self::override_join_type(&j, jt);
                        }
                        self.joins.push(j);
                        self.joined_aliases.insert(current_alias.clone());
                    } else if let Some(path) = self.find_relation_path(p_alias, &current_alias) {
                        self.auto_join_path(&path, source.join_type.as_deref())?;
                    } else {
                        return Err(format!("No connection for {}->{}", p_alias, &current_alias));
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
        
        let mut local_aliases = std::collections::HashMap::new();
        for child in &node.children {
            if child.flatten && !child.is_list {
                let c_alias = child.source.as_ref().map(|s| s.table_name.as_str()).unwrap_or(child.name.as_str());
                for (f_k, f_v) in &child.fields {
                    if f_v != "*" {
                        let expanded_f_v = self.guard.expand_mapped_fields(f_v, c_alias);
                        let processed = Guard::auto_prefix_field(&expanded_f_v, c_alias, None);
                        local_aliases.insert(f_k.clone(), processed);
                    }
                }
            }
        }
        
        let local_aliases_opt = if local_aliases.is_empty() { None } else { Some(&local_aliases) };
        let has_star = node.fields.values().any(|v| v == "*");
        let mut json_args = Vec::new();
        
        for (field_key, field_sql) in &node.fields {
            if field_sql == "*" {
                let mut expanded = Vec::new();
                let mut use_rtj = false;
                if let Some(wl) = &self.guard.whitelist {
                    if let Some(rule) = wl.get(&current_alias) {
                        if rule.is_allowed("*") { use_rtj = true; }
                        else {
                            match rule {
                                crate::guard::WhitelistRule::Mapping(map) => for (k, v) in map { expanded.push((k.clone(), v.clone())); },
                                crate::guard::WhitelistRule::Allowed(set) => for k in set { expanded.push((k.clone(), k.clone())); }
                            }
                        }
                    } else { return Err(format!("Table '{}' not in whitelist", current_alias)); }
                } else { use_rtj = true; }

                if use_rtj { json_args.push(format!("'{}', row_to_json({})", escape_sql_key(field_key), current_alias)); }
                else {
                    for (k, sql_val) in expanded {
                        let exp = self.guard.expand_mapped_fields(&sql_val, &current_alias);
                        let proc = Guard::auto_prefix_field(&exp, &current_alias, local_aliases_opt);
                        json_args.push(format!("'{}', {}", escape_sql_key(&k), proc));
                    }
                }
            } else {
                self.guard.validate_field(&current_alias, field_sql, local_aliases_opt)?;
                let exp = self.guard.expand_mapped_fields(field_sql, &current_alias);
                let proc = Guard::auto_prefix_field(&exp, &current_alias, local_aliases_opt);
                json_args.push(format!("'{}', {}", escape_sql_key(field_key), proc));
            }
        }
        
        // --- PRIORITY JOIN RESOLUTION ---
        let mut join_resolutions: std::collections::HashMap<String, (Option<String>, u8)> = std::collections::HashMap::new();
        for child in &node.children {
            if let (Some(src), false) = (&child.source, child.is_list) {
                let priority = if !src.from_macro && src.join_type.is_some() { 1 } 
                              else if src.from_macro && src.join_type.is_some() { 2 } 
                              else { 3 };
                let entry = join_resolutions.entry(src.table_name.clone()).or_insert((src.join_type.clone(), priority));
                if priority < entry.1 { *entry = (src.join_type.clone(), priority); }
            }
        }

        // Process children
        for child in &node.children {
            let mut nc = child.clone();
            if let Some(src) = &mut nc.source {
                if let Some((best_jt, _)) = join_resolutions.get(&src.table_name) {
                    if best_jt.is_some() { src.join_type = best_jt.clone(); }
                }
            }

            if nc.is_list {
                let list_result = self.build_lateral_subquery(&nc, &current_alias, &current_real)?;
                let lateral_alias = format!("{}_list", nc.name);
                self.joins.push(format!("LEFT JOIN LATERAL (\n  {}\n) {} ON true", list_result.sql, lateral_alias));
                json_args.push(format!("'{}', {}.array_data", escape_sql_key(&nc.name), lateral_alias));
            } else {
                // If this child targets an alias already joined by a previous sibling, 
                // but this child has the BETTER join type, we should have processed it first.
                // To solve this simply: if child targets a shared alias and is Priority 1, 
                // we should "inject" its join before others.
                
                // WAIT! A much simpler fix: just sort children by priority before processing.
                // But we need to keep output JSON in same order. 
                // So we'll process them in original order, but the FIRST time an alias is joined, 
                // it will use the Best Join Type discovered in the pre-scan above.
            
                let child_args = self.process_node(&nc, Some((&current_alias, &current_real)))?;
                if nc.flatten && has_star { json_args.extend(child_args); }
                else if !nc.flatten { json_args.push(format!("'{}', json_build_object({})", escape_sql_key(&nc.name), child_args.join(", "))); }
            }
        }
        Ok(json_args)
    }

    pub(crate) fn build_lateral_subquery(&mut self, node: &QueryNode, parent_alias: &str, parent_real: &str) -> Result<LateralResult, String> {
        let source = node.source.as_ref().ok_or("List node must have @source")?;
        let real_table = self.guard.resolve_alias(&source.table_name)?;
        let child_alias = &source.table_name;
        
        let mut inner_args = Vec::new();
        let mut inner_aliases = std::collections::HashSet::new();
        inner_aliases.insert(child_alias.to_string());
        
        let old_joins = std::mem::take(&mut self.joins);
        let old_wheres = std::mem::take(&mut self.wheres);
        let old_froms = std::mem::take(&mut self.froms);
        let old_aliases = std::mem::replace(&mut self.joined_aliases, inner_aliases);
        
        // Children processing with same priority logic
        let mut join_resolutions: std::collections::HashMap<String, (Option<String>, u8)> = std::collections::HashMap::new();
        for child in &node.children {
            if let (Some(src), false) = (&child.source, child.is_list) {
                let priority = if !src.from_macro && src.join_type.is_some() { 1 } else if src.from_macro && src.join_type.is_some() { 2 } else { 3 };
                let entry = join_resolutions.entry(src.table_name.clone()).or_insert((src.join_type.clone(), priority));
                if priority < entry.1 { *entry = (src.join_type.clone(), priority); }
            }
        }

        for child in &node.children {
            let mut nc = child.clone();
            if let Some(src) = &mut nc.source {
                if let Some((best_jt, _)) = join_resolutions.get(&src.table_name) {
                    if best_jt.is_some() { src.join_type = best_jt.clone(); }
                }
            }
            let child_args = self.process_node(&nc, Some((child_alias, &real_table)))?;
            if nc.flatten { inner_args.extend(child_args); }
            else { inner_args.push(format!("'{}', json_build_object({})", escape_sql_key(&nc.name), child_args.join(", "))); }
        }
        
        let inner_joins = std::mem::replace(&mut self.joins, old_joins);
        let inner_wheres = std::mem::replace(&mut self.wheres, old_wheres);
        self.froms = old_froms;
        self.joined_aliases = old_aliases;
        
        let json_obj = format!("json_build_object({})", inner_args.join(", "));
        let join_condition = if let Some(j) = &node.join { extract_on_condition(j)? }
        else {
            let res = self.resolve_relation(parent_alias, child_alias, parent_real, &real_table, &node.name)?;
            match res { Some(r) => extract_on_condition(&r)?, None => return Err(format!("No rel for {}->{}", parent_alias, child_alias)) }
        };
        let mut where_parts = vec![join_condition];
        for filter in &source.filters { where_parts.push(self.build_condition(child_alias, filter)?); }
        where_parts.extend(inner_wheres);
        
        let mut inner_sql = format!("SELECT {} AS item\n    FROM {} AS {}", json_obj, real_table, child_alias);
        for ij in &inner_joins { inner_sql.push_str(&format!("\n    {}", ij)); }
        inner_sql.push_str(&format!("\n    WHERE {}", where_parts.join(" AND ")));
        
        if let Some(order) = &source.order {
            if self.guard.is_safe_order_by(order).is_ok() {
                let prefixed = Guard::auto_prefix_field(order, child_alias, None);
                inner_sql.push_str(&format!("\n    ORDER BY {}", prefixed));
            }
        }
        if let Some(limit) = source.limit { inner_sql.push_str(&format!("\n    LIMIT {}", limit)); }
        if let Some(offset) = source.offset { inner_sql.push_str(&format!("\n    OFFSET {}", offset)); }
        
        let sql = format!("SELECT COALESCE(json_agg(sub.item), '[]'::json) AS array_data\n  FROM (\n    {}\n  ) sub", inner_sql);
        Ok(LateralResult { sql })
    }
}
