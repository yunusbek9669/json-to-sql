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

    pub fn generate(mut self, root: QueryNode) -> Result<ParseResult, String> {
        let child_args = self.process_node(&root, None)?;
        let json_obj_expr = format!("json_build_object({})", child_args.join(", "));

        // BASE SQL Construction
        let mut base_sql = String::new();
        base_sql.push_str("SELECT ");
        base_sql.push_str(&format!("{} AS uaq_data", json_obj_expr));

        // When a list root has a LIMIT and non-LATERAL JOINs exist, LIMIT must be pushed
        // into a subquery on the root table first. Otherwise each JOIN-multiplied row counts
        // toward the limit — one parent with N children consumes N of the limit slots,
        // causing the same parent row to appear N times in the result.
        let has_regular_joins = self.joins.iter().any(|j| !j.contains("LATERAL"));
        let limit_opt  = root.source.as_ref().and_then(|s| s.limit);
        let offset_opt = root.source.as_ref().and_then(|s| s.offset);
        let order_opt  = root.source.as_ref().and_then(|s| s.order.clone());
        let use_root_subquery = root.is_list
            && has_regular_joins
            && limit_opt.is_some()
            && !self.froms.is_empty();

        if use_root_subquery {
            let root_alias = root.source.as_ref().map(|s| s.table_name.as_str()).unwrap_or("");
            let root_prefix = format!("{}.", root_alias);

            // Conditions that reference the root table go into the subquery so the LIMIT
            // is applied after filtering but before joining. Conditions that reference
            // joined tables stay in the outer WHERE.
            let (root_wheres, join_wheres): (Vec<String>, Vec<String>) =
                std::mem::take(&mut self.wheres)
                    .into_iter()
                    .partition(|w| w.starts_with(&root_prefix));

            // ROW_NUMBER() assigns a unique integer to every root row before the LIMIT.
            // After the LIMIT the numbers are 1..N (one per distinct root entity).
            // The outer DISTINCT ON then collapses the JOIN-multiplied rows back to one
            // per root entity — whichever join row PostgreSQL returns first is kept.
            let mut root_sub = format!("SELECT *, ROW_NUMBER() OVER () AS _uaq_rn FROM {}", self.froms[0]);
            if !root_wheres.is_empty() {
                root_sub.push_str(&format!("\n    WHERE {}", root_wheres.join(" AND ")));
            }
            if let Some(order) = &order_opt {
                if self.guard.is_safe_order_by(order).is_ok() {
                    let prefixed = Guard::auto_prefix_field(order, root_alias, None);
                    root_sub.push_str(&format!("\n    ORDER BY {}", prefixed));
                }
            }
            root_sub.push_str(&format!("\n    LIMIT {}", limit_opt.unwrap()));
            if let Some(offset) = offset_opt {
                root_sub.push_str(&format!("\n    OFFSET {}", offset));
            }

            // Rewrite the SELECT to use DISTINCT ON so one row is kept per root entity.
            // DISTINCT ON requires ORDER BY to begin with the same expression.
            base_sql = format!(
                "SELECT DISTINCT ON ({alias}._uaq_rn) {expr} AS uaq_data",
                alias = root_alias,
                expr = json_obj_expr
            );
            base_sql.push_str(&format!("\nFROM (\n  {}\n) AS {}", root_sub, root_alias));
            if !self.joins.is_empty() {
                base_sql.push_str("\n");
                base_sql.push_str(&self.joins.join("\n"));
            }
            if !join_wheres.is_empty() {
                base_sql.push_str("\nWHERE ");
                base_sql.push_str(&join_wheres.join(" AND "));
            }
            base_sql.push_str(&format!("\nORDER BY {}._uaq_rn", root_alias));
        } else {
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
            if let Some(source) = &root.source {
                let _root_real = self.guard.resolve_alias(&source.table_name)?;
                let root_alias = &source.table_name;
                if let Some(order) = &source.order {
                    if self.guard.is_safe_order_by(order).is_ok() {
                        let prefixed_order = Guard::auto_prefix_field(order, root_alias, None);
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

        Ok(ParseResult {
            is_ok: true,
            sql: Some(final_sql),
            params: Some(self.params),
            message: "success".to_string(),
        })
    }
}