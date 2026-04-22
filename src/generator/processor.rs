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

            // Skip the JOIN when every field in this node is a local aggregate subquery
            // and there are no source-level filters — the subqueries carry their own
            // correlation condition, so a JOIN in the main query is unnecessary and
            // would multiply rows (one per related record) causing duplicate results.
            let skip_join = context.is_some()
                && !node.is_list
                && node.children.is_empty()
                && source.filters.is_empty()
                && self.all_fields_local_agg_check(&node.fields);

            if context.is_none() {
                self.froms.push(format!("{} AS {}", current_real, current_alias));
                self.joined_aliases.insert(current_alias.clone());
            } else if !node.is_list && !skip_join {
                if !self.joined_aliases.contains(&current_alias) {
                    let (p_alias, p_real) = context.unwrap();
                    let is_numeric = node.name.chars().all(|c| c.is_numeric());
                    let rel_hint = if is_numeric { &source.table_name } else { &node.name };
                    let rel_name = source.rel.as_deref().unwrap_or(rel_hint);
                    let j_str = self.resolve_relation(p_alias, &current_alias, p_real, &current_real, rel_name)?;

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
            if (context.is_none() || !node.is_list) && !skip_join {
                for filter in &source.filters {
                    let condition = self.build_condition(&current_alias, filter, Some(&node.fields))?;
                    self.wheres.push(condition);
                }
            }
        }
        
        let mut local_aliases = std::collections::HashMap::new();
        let mut processed_children = std::collections::HashMap::new();

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

        // --- PASS 1: Child Processing ---
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
                processed_children.insert(nc.name.clone(), (format!("{}.array_data", lateral_alias), false));
            } else {
                let child_args = self.process_node(&nc, Some((&current_alias, &current_real)))?;
                let c_alias = nc.source.as_ref().map(|s| s.table_name.as_str()).unwrap_or(nc.name.as_str());
                
                if nc.flatten {
                    for (f_k, f_v) in &nc.fields {
                        if f_v != "*" {
                            let expanded_f_v = self.guard.expand_mapped_fields(f_v, c_alias);
                            let processed = Guard::auto_prefix_field(&expanded_f_v, c_alias, None);
                            local_aliases.insert(f_k.clone(), processed);
                        }
                    }
                    processed_children.insert(nc.name.clone(), (child_args.join(", "), true));
                } else {
                    let child_sql = format!("json_build_object({})", child_args.join(", "));
                    local_aliases.insert(nc.name.clone(), child_sql.clone());
                    processed_children.insert(nc.name.clone(), (child_sql, false));
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
            } else if let Some((parent_col, child_col, fields, is_json)) = self.try_parse_parents_local(field_sql) {
                let col_ref = self.build_parents_local(&parent_col, &child_col, &fields, is_json, &current_alias, &current_real)?;
                json_args.push(format!("'{}', {}", escape_sql_key(field_key), col_ref));
            } else if let Some((func, filter_str, col)) = self.try_parse_local_agg(field_sql) {
                let inline_sql = self.build_local_inline_agg(&func, filter_str, col, &current_alias, &current_real, context)?;
                json_args.push(format!("'{}', ({})", escape_sql_key(field_key), inline_sql));
            } else if let Some((func, src_part, fld_part)) = self.try_parse_inline_agg(field_sql) {
                let inline_sql = self.build_inline_agg(&func, &src_part, fld_part, &current_alias, &current_real)?;
                json_args.push(format!("'{}', ({})", escape_sql_key(field_key), inline_sql));
            } else {
                self.guard.validate_field(&current_alias, field_sql, local_aliases_opt)?;
                let exp = self.guard.expand_mapped_fields(field_sql, &current_alias);
                let proc = Guard::auto_prefix_field(&exp, &current_alias, local_aliases_opt);
                json_args.push(format!("'{}', {}", escape_sql_key(field_key), proc));
            }
        }
        
        // --- FINAL ASSEMBLY ---
        let mut used_children = std::collections::HashSet::new();
        // Identify which non-list, non-flattened children were already used in @fields
        for (_, field_sql) in &node.fields {
            if let Some((name, _)) = processed_children.iter().find(|(k, (_, flat))| !*flat && field_sql == *k) {
                used_children.insert(name.clone());
            }
        }

        if has_star || node.fields.is_empty() {
            // Include everything from children that wasn't flattened OR was flattened but user asked for '*'
            for child in &node.children {
                if let Some((child_sql, is_flat)) = processed_children.get(&child.name) {
                    if *is_flat {
                        if (!child.from_macro || has_star) && !child_sql.is_empty() {
                            json_args.push(child_sql.clone());
                        }
                    } else if !used_children.contains(&child.name) {
                        json_args.push(format!("'{}', {}", escape_sql_key(&child.name), child_sql));
                    }
                }
            }
        } else {
            // Strict selection: only automatically include explicit nodes (not from macro)
            for child in &node.children {
                if let Some((child_sql, is_flat)) = processed_children.get(&child.name) {
                    if *is_flat {
                        if !child.from_macro && !child_sql.is_empty() {
                            json_args.push(child_sql.clone());
                        }
                    } else if !child.from_macro && !used_children.contains(&child.name) {
                        json_args.push(format!("'{}', {}", escape_sql_key(&child.name), child_sql));
                    }
                }
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
        
        
        for (field_key, field_sql) in &node.fields {
            if field_sql == "*" {
                let mut expanded = Vec::new();
                let mut use_rtj = false;
                if let Some(wl) = &self.guard.whitelist {
                    if let Some(rule) = wl.get(child_alias) {
                        if rule.is_allowed("*") { use_rtj = true; }
                        else {
                            match rule {
                                crate::guard::WhitelistRule::Mapping(map) => for (k, v) in map { expanded.push((k.clone(), v.clone())); },
                                crate::guard::WhitelistRule::Allowed(set) => for k in set { expanded.push((k.clone(), k.clone())); }
                            }
                        }
                    } else { return Err(format!("Table '{}' not in whitelist", child_alias)); }
                } else { use_rtj = true; }

                if use_rtj { inner_args.push(format!("'{}', row_to_json({})", escape_sql_key(field_key), child_alias)); }
                else {
                    for (k, sql_val) in expanded {
                        let exp = self.guard.expand_mapped_fields(&sql_val, child_alias);
                        let proc = Guard::auto_prefix_field(&exp, child_alias, None);
                        inner_args.push(format!("'{}', {}", escape_sql_key(&k), proc));
                    }
                }
            } else if let Some((parent_col, child_col, fields, is_json)) = self.try_parse_parents_local(field_sql) {
                let col_ref = self.build_parents_local(&parent_col, &child_col, &fields, is_json, child_alias, &real_table)?;
                inner_args.push(format!("'{}', {}", escape_sql_key(field_key), col_ref));
            } else if let Some((func, filter_str, col)) = self.try_parse_local_agg(field_sql) {
                let inline_sql = self.build_local_inline_agg(&func, filter_str, col, child_alias, &real_table, Some((parent_alias, parent_real)))?;
                inner_args.push(format!("'{}', ({})", escape_sql_key(field_key), inline_sql));
            } else if let Some((func, src_part, fld_part)) = self.try_parse_inline_agg(field_sql) {
                let inline_sql = self.build_inline_agg(&func, &src_part, fld_part, child_alias, &real_table)?;
                inner_args.push(format!("'{}', ({})", escape_sql_key(field_key), inline_sql));
            } else {
                self.guard.validate_field(child_alias, field_sql, None)?;
                let exp = self.guard.expand_mapped_fields(field_sql, child_alias);
                let proc = Guard::auto_prefix_field(&exp, child_alias, None);
                inner_args.push(format!("'{}', {}", escape_sql_key(field_key), proc));
            }
        }
        
        let inner_joins = std::mem::replace(&mut self.joins, old_joins);
        let inner_wheres = std::mem::replace(&mut self.wheres, old_wheres);
        self.froms = old_froms;
        self.joined_aliases = old_aliases;
        
        let json_obj = format!("json_build_object({})", inner_args.join(", "));
        let join_condition = {
            let res = self.resolve_relation(parent_alias, child_alias, parent_real, &real_table, &node.name)?;
            match res { Some(r) => extract_on_condition(&r)?, None => return Err(format!("No rel for {}->{}", parent_alias, child_alias)) }
        };
        let mut where_parts = vec![join_condition];
        for filter in &source.filters { where_parts.push(self.build_condition(child_alias, filter, Some(&node.fields))?); }
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

    fn all_fields_local_agg_check(&self, fields: &indexmap::IndexMap<String, String>) -> bool {
        !fields.is_empty() && fields.values().all(|v| self.try_parse_local_agg(v).is_some())
    }

    pub(crate) fn try_parse_inline_agg(&self, field_sql: &str) -> Option<(String, String, Option<String>)> {
        let text = field_sql.trim();
        let lower = text.to_lowercase();
        let func = if lower.starts_with("count(") { "COUNT" }
                   else if lower.starts_with("sum(") { "SUM" }
                   else if lower.starts_with("max(") { "MAX" }
                   else if lower.starts_with("min(") { "MIN" }
                   else if lower.starts_with("avg(") { "AVG" }
                   else { return None; };

        if !text.ends_with(')') {
            return None;
        }

        let inner = text[text.find('(').unwrap() + 1..text.len() - 1].trim();

        let mut source_part = inner;
        let mut field_part = None;

        if let Some(bracket_end) = inner.rfind(']') {
            if let Some(dot_idx) = inner[bracket_end..].find('.') {
                let actual_dot = bracket_end + dot_idx;
                source_part = &inner[..actual_dot];
                field_part = Some(inner[actual_dot + 1..].trim().to_string());
            }
        } else if let Some(dot_idx) = inner.find('.') {
            source_part = &inner[..dot_idx];
            field_part = Some(inner[dot_idx + 1..].trim().to_string());
        }

        let child_alias = source_part.split('[').next().unwrap_or(source_part).trim();
        
        let mut is_agg = source_part.contains('[');
        if !is_agg {
            if let Some(wl) = &self.guard.whitelist {
                if wl.contains_key(child_alias) {
                    is_agg = true;
                }
            }
        }

        if is_agg {
            return Some((func.to_string(), source_part.to_string(), field_part));
        }

        None
    }

    pub(crate) fn build_inline_agg(
        &mut self,
        func_upper: &str,
        source_part: &str,
        field_part: Option<String>,
        parent_alias: &str,
        parent_real: &str,
    ) -> Result<String, String> {
        let src = crate::parser::parse_source(source_part);
        self.guard.validate_table(&src.table_name)?;
        let child_real = self.guard.resolve_alias(&src.table_name)?;
        let child_alias = &src.table_name;

        if func_upper != "COUNT" && field_part.is_none() {
            return Err(format!("Field is required for {} aggregation", func_upper));
        }

        let agg_expr = if let Some(f) = &field_part {
            self.guard.validate_column(child_alias, f)?;
            let expanded = self.guard.expand_mapped_fields(f, child_alias);
            let prefixed = Guard::auto_prefix_field(&expanded, child_alias, None);
            format!("{}({})", func_upper, prefixed)
        } else {
            "COUNT(*)".to_string()
        };

        let rel_result = self.resolve_relation(parent_alias, child_alias, parent_real, &child_real, child_alias)?;
        let on_condition = match rel_result {
            Some(r) => extract_on_condition(&r)?,
            None => {
                if let Some(path) = self.find_relation_path(parent_alias, child_alias) {
                    if path.len() == 2 {
                        let from_real = self.guard.resolve_alias(&path[0])?;
                        let to_real   = self.guard.resolve_alias(&path[1])?;
                        let r = self.resolve_relation(&path[0], &path[1], &from_real, &to_real, &path[1])?
                            .ok_or_else(|| format!("No relation for inline agg {}->{}", path[0], path[1]))?;
                        extract_on_condition(&r)?
                    } else {
                        return Err(format!("Inline agg: multi-hop path not supported for {}->{}", parent_alias, child_alias));
                    }
                } else {
                    return Err(format!("No relation found for inline agg: {}->{}", parent_alias, child_alias));
                }
            }
        };

        let mut extra_wheres = Vec::new();
        for filter in &src.filters {
            let cond = self.build_condition(child_alias, filter, None)?;
            extra_wheres.push(cond);
        }

        let table_expr = if *child_real == *child_alias {
            child_real.clone()
        } else {
            format!("{} AS {}", child_real, child_alias)
        };

        let mut where_clause = on_condition;
        if !extra_wheres.is_empty() {
            where_clause = format!("{} AND {}", where_clause, extra_wheres.join(" AND "));
        }

        Ok(format!("SELECT {} FROM {} WHERE {}", agg_expr, table_expr, where_clause))
    }

    // Returns (parent_col, child_col, fields: Vec<(output_key, column_name)>, is_json)
    //
    // Supported field syntaxes (3rd argument):
    //   [name]          → JSON array, key = column name         → [{"name": "..."}]
    //   [name, code]    → JSON array, multiple columns          → [{"name": "...", "code": "..."}]
    //   {nn:name}       → JSON array, custom key for column     → [{"nn": "..."}]
    //   {nn:name,kk:id} → JSON array, multiple custom mappings  → [{"nn": "...", "kk": "..."}]
    //   name            → plain string (string_agg)             → "root, ..., current"
    pub(crate) fn try_parse_parents_local(&self, field_sql: &str) -> Option<(String, String, Vec<(String, String)>, bool)> {
        let text = field_sql.trim();
        let lower = text.to_lowercase();
        if !lower.starts_with("parents(") || !lower.ends_with(')') {
            return None;
        }
        let inner = text[8..text.len() - 1].trim();

        // Split top-level args by comma, respecting bracket depth ([, {, ()
        let mut parts: Vec<String> = Vec::new();
        let mut current_part = String::new();
        let mut depth = 0i32;

        for c in inner.chars() {
            if c == '[' || c == '{' || c == '(' { depth += 1; }
            else if c == ']' || c == '}' || c == ')' { depth -= 1; }

            if c == ',' && depth == 0 {
                parts.push(current_part.trim().to_string());
                current_part.clear();
            } else {
                current_part.push(c);
            }
        }
        if !current_part.trim().is_empty() {
            parts.push(current_part.trim().to_string());
        }

        if parts.len() != 3 {
            return None;
        }

        let parent_col  = parts[0].trim().to_string();
        let child_col   = parts[1].trim().to_string();
        let fields_str  = parts[2].trim().to_string();

        // FIX #8: JSON output key in {key:col} must be a safe identifier.
        static KEY_RE: once_cell::sync::Lazy<regex::Regex> =
            once_cell::sync::Lazy::new(|| regex::Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*$").unwrap());

        let (fields, is_json) = if fields_str.starts_with('{') && fields_str.ends_with('}') {
            // {nn:name, kk:code} → custom key mapping, always JSON array
            let inner = &fields_str[1..fields_str.len() - 1];
            let pairs: Vec<(String, String)> = inner.split(',')
                .filter_map(|p| {
                    let mut kv = p.splitn(2, ':');
                    let key = kv.next()?.trim().to_string();
                    let col = kv.next()?.trim().to_string();
                    // Key must be a valid identifier; silently drop invalid pairs.
                    if key.is_empty() || col.is_empty() || !KEY_RE.is_match(&key) { return None; }
                    Some((key, col))
                })
                .collect();
            if pairs.is_empty() { return None; }
            (pairs, true)
        } else if fields_str.starts_with('[') && fields_str.ends_with(']') {
            // [name, code] → column name is also the output key
            let inner = &fields_str[1..fields_str.len() - 1];
            let pairs: Vec<(String, String)> = inner.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(|s| (s.clone(), s))
                .collect();
            if pairs.is_empty() { return None; }
            (pairs, true)
        } else {
            // bare column name → string_agg output
            let col = fields_str.clone();
            (vec![(col.clone(), col)], false)
        };

        Some((parent_col, child_col, fields, is_json))
    }

    // Generates a LATERAL JOIN for the parents() traversal and returns the column reference
    // (e.g. "_plat1.result") to embed in json_build_object.
    //
    // WHY LATERAL instead of a scalar subquery:
    //   PostgreSQL does not guarantee that a correlated reference inside a WITH RECURSIVE
    //   scalar subquery is re-evaluated per outer row — in practice the CTE can be
    //   materialised once, making `outer_alias.id` resolve to a stale or wrong value and
    //   causing a full-table scan.  A LATERAL subquery is explicitly re-evaluated for each
    //   row of the driving table, so the correlated reference is always correct.
    pub(crate) fn build_parents_local(
        &mut self,
        parent_col: &str,
        child_col: &str,
        fields: &[(String, String)],   // (output_key, column_name)
        is_json: bool,
        current_alias: &str,
        current_real: &str,
    ) -> Result<String, String> {
        self.guard.validate_column(current_alias, parent_col)?;
        self.guard.validate_column(current_alias, child_col)?;
        for (_, col) in fields { self.guard.validate_column(current_alias, col)?; }

        let ba  = format!("{}_base", current_alias);
        let ra  = format!("{}_r",    current_alias);
        let cte = format!("{}_tree", current_alias);

        self.param_counter += 1;
        let lat = format!("_plat{}", self.param_counter);

        let mut base_sel: Vec<String> = Vec::new();
        let mut rec_sel:  Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Include parent_col and child_col for the recursive join, then the output columns.
        for col in std::iter::once(parent_col)
            .chain(std::iter::once(child_col))
            .chain(fields.iter().map(|(_, c)| c.as_str()))
        {
            if seen.insert(col.to_string()) {
                let exp = self.guard.expand_mapped_fields(col, current_alias);
                base_sel.push(format!("{} AS {}", Guard::auto_prefix_field(&exp, &ba, None), col));
                rec_sel .push(format!("{} AS {}", Guard::auto_prefix_field(&exp, &ra, None), col));
            }
        }

        // ORDER BY _depth DESC → root first in output (depth 1 = current, depth N = root).
        let agg = if is_json {
            let parts: Vec<String> = {
                let mut seen_keys = std::collections::HashSet::new();
                fields.iter()
                    .filter(|(k, _)| seen_keys.insert(k.to_string()))
                    .map(|(key, col)| format!("'{}', {}", escape_sql_key(key), col))
                    .collect()
            };
            format!("COALESCE(json_agg(json_build_object({}) ORDER BY _depth DESC), '[]'::json)", parts.join(", "))
        } else {
            if fields.len() > 1 {
                return Err("parents() string format supports only 1 field".to_string());
            }
            format!("COALESCE(string_agg({}::text, ', ' ORDER BY _depth DESC), '')", fields[0].1)
        };

        // Canonical recursive form: CTE reference appears first in FROM so PostgreSQL's
        // recursive evaluation substitutes only the working table, not the full table.
        // `parent_id IS NOT NULL` explicitly terminates at the root without relying solely
        // on a failed JOIN (handles both NULL and missing-row cases more clearly).
        let lateral = format!(
            concat!(
                "LEFT JOIN LATERAL (\n",
                "  WITH RECURSIVE {cte} AS (\n",
                "    SELECT {base_sel}, 1 AS _depth\n",
                "    FROM {real} AS {ba}\n",
                "    WHERE {ba}.{id} = {alias}.{id}\n",
                "    UNION ALL\n",
                "    SELECT {rec_sel}, {cte}._depth + 1 AS _depth\n",
                "    FROM {cte}\n",
                "    JOIN {real} AS {ra} ON {ra}.{id} = {cte}.{pid}\n",
                "      AND {cte}.{pid} IS NOT NULL\n",
                "      AND {cte}._depth < 50\n",
                "  )\n",
                "  SELECT {agg} AS result FROM {cte}\n",
                ") {lat} ON true"
            ),
            cte      = cte,
            base_sel = base_sel.join(", "),
            real     = current_real,
            ba       = ba,
            id       = child_col,
            alias    = current_alias,
            pid      = parent_col,
            rec_sel  = rec_sel.join(", "),
            ra       = ra,
            agg      = agg,
            lat      = lat,
        );

        self.joins.push(lateral);
        Ok(format!("{}.result", lat))
    }

    pub(crate) fn try_parse_local_agg(&self, field_sql: &str) -> Option<(String, Option<String>, Option<String>)> {
        let text = field_sql.trim();
        let lower = text.to_lowercase();
        let func = if lower.starts_with("count(") { "COUNT" }
                   else if lower.starts_with("sum(") { "SUM" }
                   else if lower.starts_with("max(") { "MAX" }
                   else if lower.starts_with("min(") { "MIN" }
                   else if lower.starts_with("avg(") { "AVG" }
                   else { return None; };

        if !text.ends_with(')') { return None; }
        
        let inner = text[text.find('(').unwrap() + 1..text.len() - 1].trim();

        if inner == "*" {
            return Some((func.to_string(), None, None));
        }

        let mut filter = None;
        let mut col = None;

        if inner.starts_with('[') {
            if let Some(close_idx) = inner.find(']') {
                filter = Some(inner[1..close_idx].to_string());
                let rem = inner[close_idx+1..].trim();
                if rem.starts_with('.') {
                    col = Some(rem[1..].trim().to_string());
                } else if !rem.is_empty() {
                    col = Some(rem.to_string());
                }
            } else { return None; }
        } else {
            let first_token = inner.split('.').next().unwrap_or(inner);
            let is_in_whitelist = self.guard.whitelist.as_ref().map(|wl| wl.contains_key(first_token)).unwrap_or(false);
            if is_in_whitelist {
                return None;
            }
            col = Some(inner.to_string());
        }

        Some((func.to_string(), filter, col))
    }

    pub(crate) fn build_local_inline_agg(
        &mut self,
        func: &str,
        filter_str: Option<String>,
        col: Option<String>,
        current_alias: &str,
        current_real: &str,
        context: Option<(&str, &str)>,
    ) -> Result<String, String> {
        let (parent_alias, parent_real) = context.ok_or_else(|| format!("Cannot use local aggregate {} at root level", func))?;

        if func != "COUNT" && col.is_none() {
            return Err(format!("Column is required for local aggregate {}", func));
        }

        let agg_expr = if let Some(c) = col {
            self.guard.validate_column(current_alias, &c)?;
            let expanded = self.guard.expand_mapped_fields(&c, current_alias);
            let prefixed = Guard::auto_prefix_field(&expanded, current_alias, None);
            format!("{}({})", func, prefixed)
        } else {
            "COUNT(*)".to_string()
        };

        let rel_result = self.resolve_relation(parent_alias, current_alias, parent_real, current_real, current_alias)?;
        let on_condition = match rel_result {
            Some(r) => extract_on_condition(&r)?,
            None => {
                if let Some(path) = self.find_relation_path(parent_alias, current_alias) {
                    if path.len() == 2 {
                        let from_real = self.guard.resolve_alias(&path[0])?;
                        let to_real   = self.guard.resolve_alias(&path[1])?;
                        let r = self.resolve_relation(&path[0], &path[1], &from_real, &to_real, &path[1])?
                            .ok_or_else(|| format!("No relation for local agg {}->{}", path[0], path[1]))?;
                        extract_on_condition(&r)?
                    } else {
                        return Err(format!("Local agg: multi-hop path not supported for {}->{}", parent_alias, current_alias));
                    }
                } else {
                    return Err(format!("No relation found for local agg: {}->{}", parent_alias, current_alias));
                }
            }
        };

        let mut extra_wheres = Vec::new();
        if let Some(f) = filter_str {
           let mock_source = format!("{}[{}]", current_alias, f);
           let src = crate::parser::parse_source(&mock_source);
           for filter in &src.filters {
               let cond = self.build_condition(current_alias, filter, None)?;
               extra_wheres.push(cond);
           }
        }

        let mut where_clause = on_condition;
        if !extra_wheres.is_empty() {
            where_clause = format!("{} AND {}", where_clause, extra_wheres.join(" AND "));
        }

        let table_expr = if current_real == current_alias {
            current_real.to_string()
        } else {
            format!("{} AS {}", current_real, current_alias)
        };

        Ok(format!("SELECT {} FROM {} WHERE {}", agg_expr, table_expr, where_clause))
    }
}

