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
    pub fn new(whitelist: Option<HashMap<String, serde_json::Value>>, relations: Option<HashMap<String, String>>) -> Self {
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
            // Validate alias resolves, but use alias for column prefix
            let _root_real = self.guard.resolve_alias(&source.table_name)?;
            let root_alias = &source.table_name;
            if let Some(order) = &source.order {
                if self.guard.is_safe_order_by(order).is_ok() {
                    let prefixed_order = Guard::auto_prefix_field(order, root_alias);
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
}
