use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRule {
    pub field: String,
    pub operator: String, // "eq", "neq", "gt", "lt", "between", "in", "like"
    pub value: String,    // Stored as string to be parsed or inserted directly as bound param
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceDef {
    pub table_name: String,
    pub filters: Vec<FilterRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryNode {
    pub name: String,
    pub source: Option<SourceDef>,
    pub join: Option<String>,
    pub fields: HashMap<String, String>,
    pub children: Vec<QueryNode>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootQuery {
    pub nodes: Vec<QueryNode>,
    pub config: GlobalConfig,
}

#[derive(Debug, Serialize)]
pub struct ParseResult {
    pub sql: String,
    pub params: HashMap<String, Value>,
    pub structure: Value,
}
