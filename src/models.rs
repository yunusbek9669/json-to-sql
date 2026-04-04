use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRule {
    pub field: String,
    pub operator: String, // "eq", "neq", "gt", "lt", "between", "in", "like"
    pub value: String,    // Stored as string to be parsed or inserted directly as bound param
}

pub struct SourceDef {
    pub table_name: String,
    pub filters: Vec<FilterRule>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub order: Option<String>,
    /// Override JOIN type: "left", "right", "inner" (from $join modifier)
    pub join_type: Option<String>,
    /// Explicit relation name (from $rel modifier, maps to :node_name suffix)
    pub rel: Option<String>,
}

pub struct QueryNode {
    pub name: String,
    pub is_list: bool,
    pub source: Option<SourceDef>,
    pub join: Option<String>,
    pub flatten: bool,
    pub fields: IndexMap<String, String>,
    pub children: Vec<QueryNode>,
    /// Query mode: "list" (default), "count", "first"
    pub mode: Option<String>,
}



#[derive(Debug, Serialize)]
pub struct ParseResult {
    #[serde(rename = "isOk")]
    pub is_ok: bool,
    pub sql: Option<String>,
    pub params: Option<IndexMap<String, Value>>,
    pub structure: Option<Value>,
    pub message: String,
}
