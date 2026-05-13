pub mod processor;
pub mod condition;
pub mod relation;

use std::collections::{HashMap, HashSet};
use indexmap::IndexMap;
use serde_json::Value;

use crate::models::{ParseResult, QueryNode};
use crate::guard::Guard;

pub struct SqlGenerator {
    pub(crate) param_counter: usize,
    pub(crate) params: IndexMap<String, Value>,
    pub(crate) froms: Vec<String>,
    pub(crate) joins: Vec<String>,
    pub(crate) wheres: Vec<String>,
    pub(crate) guard: Guard,
    pub(crate) relations: HashMap<String, String>,
    /// Track already-joined aliases to avoid duplicates in auto-path resolution
    pub(crate) joined_aliases: HashSet<String>,
    /// Cached adjacency graph built from relations (for BFS path discovery)
    pub(crate) relation_graph: HashMap<String, Vec<String>>,
}

impl SqlGenerator {
    pub fn new(whitelist: Option<IndexMap<String, serde_json::Value>>, relations: Option<HashMap<String, String>>) -> Self {
        let rels = relations.unwrap_or_default();
        let relation_graph = Self::build_relation_graph(&rels);
        Self {
            param_counter: 0,
            params: IndexMap::new(),
            froms: Vec::new(),
            joins: Vec::new(),
            wheres: Vec::new(),
            guard: Guard::new(whitelist),
            relations: rels,
            joined_aliases: HashSet::new(),
            relation_graph,
        }
    }

    /// Extracts the table alias from a JOIN string.
    /// "INNER JOIN real_tbl AS alias ON ..." → "alias"
    /// "INNER JOIN tbl ON ..."              → "tbl"
    fn extract_join_alias(join_str: &str) -> Option<String> {
        let trimmed = join_str.trim();
        let upper = trimmed.to_uppercase();
        let prefixes = ["INNER JOIN ", "LEFT JOIN ", "RIGHT JOIN ", "FULL JOIN ", "CROSS JOIN "];
        let offset = prefixes.iter().find_map(|p| upper.starts_with(p).then(|| p.len()))?;
        let rest = &trimmed[offset..];
        let before_on = rest.split(" ON ").next()?.trim();
        let upper_before_on = before_on.to_uppercase();
        if let Some(pos) = upper_before_on.find(" AS ") {
            Some(before_on[pos + 4..].trim().to_string())
        } else {
            Some(before_on.to_string())
        }
    }

    pub fn generate(mut self, root: QueryNode) -> Result<ParseResult, String> {
        let child_args = self.process_node(&root, None)?;
        let json_obj_expr = format!("json_build_object({})", child_args.join(", "));

        // BASE SQL Construction
        let mut base_sql = String::new();
        base_sql.push_str("SELECT ");
        base_sql.push_str(&format!("{} AS uaq_data", json_obj_expr));

        let limit_opt  = root.source.as_ref().and_then(|s| s.limit);
        let offset_opt = root.source.as_ref().and_then(|s| s.offset);
        let root_table_name = root.source.as_ref().map(|s| s.table_name.as_str()).unwrap_or("");
        let order_opt: Option<String> = root.source.as_ref()
            .and_then(|s| s.order.clone())
            .or_else(|| self.guard.get_default_order(root_table_name).map(|s| s.to_string()));

        // Split joins into two categories:
        // - filter_joins (INNER JOIN): reduce rows — must be inside the root subquery so
        //   LIMIT applies after filtering, not before. After the subquery they are re-joined
        //   in the outer query so their columns remain accessible to the SELECT list.
        // - enrich_joins (LEFT/FULL/RIGHT + LATERAL): can multiply or add rows — stay in
        //   the outer query; DISTINCT ON collapses any multiplied rows back to one per root.
        let all_joins = std::mem::take(&mut self.joins);
        let (filter_joins, enrich_joins): (Vec<String>, Vec<String>) = all_joins
            .into_iter()
            .partition(|j| {
                let u = j.trim().to_uppercase();
                !u.contains("LATERAL") && u.starts_with("INNER JOIN")
            });
        self.joins = enrich_joins;

        // Aliases of INNER JOIN tables — their WHERE conditions filter root rows
        // (needed for both subquery building and total COUNT generation).
        let filter_aliases: Vec<String> = filter_joins.iter()
            .filter_map(|j| Self::extract_join_alias(j))
            .collect();

        let has_regular_joins = self.joins.iter().any(|j| !j.contains("LATERAL"))
            || !filter_joins.is_empty();
        let use_root_subquery = root.is_list
            && has_regular_joins
            && limit_opt.is_some()
            && !self.froms.is_empty();

        // Conditions that affect root row count — populated below, used for total COUNT.
        let mut total_count_wheres: Vec<String> = Vec::new();

        if use_root_subquery {
            let root_alias = root.source.as_ref().map(|s| s.table_name.as_str()).unwrap_or("");
            let root_prefix = format!("{}.", root_alias);

            // Route WHERE conditions:
            //   root-table columns + INNER-JOIN-table columns  → subquery WHERE
            //   everything else (LEFT JOIN tables)             → outer WHERE
            let all_wheres = std::mem::take(&mut self.wheres);
            let (subq_wheres, outer_wheres): (Vec<String>, Vec<String>) =
                all_wheres.into_iter().partition(|w| {
                    w.starts_with(&root_prefix)
                        || filter_aliases.iter().any(|a| w.starts_with(&format!("{}.", a)))
                });

            total_count_wheres = subq_wheres.clone();

            // Root subquery: root table + INNER JOINs + their conditions + LIMIT.
            // Selecting only root_alias.* avoids column-name ambiguity when both tables
            // share column names such as "id" or "status".
            let mut root_sub = format!(
                "SELECT {alias}.*, ROW_NUMBER() OVER () AS _uaq_rn FROM {from}",
                alias = root_alias,
                from  = self.froms[0]
            );
            for j in &filter_joins {
                root_sub.push_str(&format!("\n    {}", j));
            }
            if !subq_wheres.is_empty() {
                root_sub.push_str(&format!("\n    WHERE {}", subq_wheres.join(" AND ")));
            }
            if let Some(order) = &order_opt {
                if self.guard.is_safe_order_by(order).is_ok() {
                    let expanded = self.guard.expand_mapped_fields(order, root_alias);
                    let prefixed = Guard::auto_prefix_field(&expanded, root_alias, None);
                    root_sub.push_str(&format!("\n    ORDER BY {}", prefixed));
                }
            }
            root_sub.push_str(&format!("\n    LIMIT {}", limit_opt.unwrap()));
            if let Some(offset) = offset_opt {
                root_sub.push_str(&format!("\n    OFFSET {}", offset));
            }

            // DISTINCT ON is only needed when enriching (LEFT/FULL) joins can multiply rows.
            let has_enrich_regular = self.joins.iter().any(|j| !j.contains("LATERAL"));
            if has_enrich_regular {
                base_sql = format!(
                    "SELECT DISTINCT ON ({alias}._uaq_rn) {expr} AS uaq_data",
                    alias = root_alias,
                    expr  = json_obj_expr
                );
            } else {
                base_sql = format!("SELECT {} AS uaq_data", json_obj_expr);
            }
            base_sql.push_str(&format!("\nFROM (\n  {}\n) AS {}", root_sub, root_alias));

            // Re-join INNER JOIN tables in the outer query so their columns are accessible
            // to the SELECT list. These re-joins are 1-to-1 (same FK, subquery already
            // guarantees exactly one matching row per root row), so no row multiplication.
            for j in &filter_joins {
                base_sql.push_str(&format!("\n{}", j));
            }
            if !self.joins.is_empty() {
                base_sql.push_str("\n");
                base_sql.push_str(&self.joins.join("\n"));
            }
            if !outer_wheres.is_empty() {
                base_sql.push_str("\nWHERE ");
                base_sql.push_str(&outer_wheres.join(" AND "));
            }
            if has_enrich_regular {
                base_sql.push_str(&format!("\nORDER BY {}._uaq_rn", root_alias));
            }
        } else {
            if !self.froms.is_empty() {
                base_sql.push_str("\nFROM ");
                base_sql.push_str(&self.froms.join(", "));
            }
            for j in &filter_joins {
                base_sql.push_str(&format!("\n{}", j));
            }
            if !self.joins.is_empty() {
                base_sql.push_str("\n");
                base_sql.push_str(&self.joins.join("\n"));
            }
            if !self.wheres.is_empty() {
                base_sql.push_str("\nWHERE ");
                base_sql.push_str(&self.wheres.join(" AND "));
            }
            // For list queries collect conditions that affect root row count
            if root.is_list {
                if let Some(source) = &root.source {
                    let root_prefix = format!("{}.", source.table_name);
                    total_count_wheres = self.wheres.iter()
                        .filter(|w| {
                            w.starts_with(&root_prefix)
                                || filter_aliases.iter().any(|a| w.starts_with(&format!("{}.", a)))
                        })
                        .cloned()
                        .collect();
                }
            }
            if let Some(source) = &root.source {
                let _root_real = self.guard.resolve_alias(&source.table_name)?;
                let root_alias = &source.table_name;
                let effective_order = source.order.as_deref()
                    .or_else(|| self.guard.get_default_order(root_alias));
                if let Some(order) = effective_order {
                    if self.guard.is_safe_order_by(order).is_ok() {
                        let expanded = self.guard.expand_mapped_fields(order, root_alias);
                        let prefixed_order = Guard::auto_prefix_field(&expanded, root_alias, None);
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
        }

        // Wrap depending on whether root node is a list
        let mut final_sql = String::new();

        if root.is_list {
            final_sql.push_str("SELECT COALESCE(json_agg(t.uaq_data), '[]'::json) \nFROM (\n");
            for line in base_sql.lines() {
                final_sql.push_str("  ");
                final_sql.push_str(line);
                final_sql.push_str("\n");
            }
            final_sql.push_str(") t");
        } else {
            // For a single object, we don't aggregate, and we limit results to 1 to ensure a single JSON object
            final_sql.push_str("SELECT t.uaq_data \nFROM (\n");
            for line in base_sql.lines() {
                final_sql.push_str("  ");
                final_sql.push_str(line);
                final_sql.push_str("\n");
            }
            // Ensuring it's only one row for single object
            if root.source.as_ref().and_then(|s| s.limit).is_none() {
                final_sql.push_str("  LIMIT 1\n");
            }
            final_sql.push_str(") t");
        }

        // Build total COUNT SQL for list queries (same filters, no LIMIT/OFFSET)
        let (total_sql, total_params) = if root.is_list {
            if let Some(source) = &root.source {
                if let Ok(root_real) = self.guard.resolve_alias(&source.table_name) {
                    let root_alias = &source.table_name;
                    let mut count_sql = format!("SELECT COUNT(*) FROM {} AS {}", root_real, root_alias);
                    for j in &filter_joins {
                        count_sql.push_str(&format!("\n{}", j));
                    }
                    if !total_count_wheres.is_empty() {
                        count_sql.push_str(&format!("\nWHERE {}", total_count_wheres.join(" AND ")));
                    }
                    // Only include params that are actually referenced in the total SQL.
                    // The main params map may contain extra params from child nodes
                    // (e.g. default filters on joined tables) that are absent from the
                    // total SQL — binding them would cause PDO HY093.
                    let referenced: IndexMap<String, Value> = self.params.iter()
                        .filter(|(key, _)| count_sql.contains(&format!(":{}", key)))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    (Some(count_sql), Some(referenced))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        Ok(ParseResult {
            is_ok: true,
            sql: Some(final_sql),
            total: total_sql,
            total_params,
            params: Some(self.params),
            message: "success".to_string(),
        })
    }
}