use std::collections::HashMap;
use serde_json::{json, Value};

pub fn process_info_request(
    info_arr: &[Value],
    whitelist_str: Option<&str>,
    relations_str: Option<&str>,
) -> Value {
    let is_tables = info_arr.iter().any(|v| v.as_str() == Some("@tables"));
    let is_relations = info_arr.iter().any(|v| v.as_str() == Some("@relations"));
    
    let mut info_result = serde_json::Map::new();
    let mut included_relations = String::from("[]");

    if is_relations && relations_str.is_some() {
        let rel_str = relations_str.unwrap_or("{}");
        let rel_map: HashMap<String, String> = serde_json::from_str(rel_str).unwrap_or_default();
        let keys: Vec<String> = rel_map.keys().cloned().collect();
        included_relations = serde_json::to_string(&keys).unwrap_or_else(|_| "[]".to_string());
        info_result.insert("relations".to_string(), json!(keys));
    }

    let mut build_args = vec![];
    if is_tables && whitelist_str.is_some() {
        build_args.push("'tables', COALESCE((SELECT tables_obj FROM tables_json), '{}'::jsonb)".to_string());
    }
    if is_relations {
        let rel_escaped = included_relations.replace("'", "''");
        build_args.push(format!("'relations', '{}'::jsonb", rel_escaped));
    }
    
    let build_args_str = if build_args.is_empty() {
        "'result', '{}'::jsonb".to_string()
    } else {
        build_args.join(",\n    ")
    };

    let sql_query = if is_tables && whitelist_str.is_some() {
        let wl_str = whitelist_str.unwrap_or("{}");
        let wl_escaped = wl_str.replace("'", "''");

        format!(r#"WITH input_json AS (
    SELECT '{}'::jsonb AS data
),
parsed_tables AS (
    SELECT 
        split_part(key, ':', 1) AS table_name,
        COALESCE(NULLIF(split_part(key, ':', 2), ''), split_part(key, ':', 1)) AS table_alias,
        value AS col_data
    FROM input_json, jsonb_each(data)
),
parsed_columns AS (
    SELECT 
        pt.table_name,
        pt.table_alias,
        obj.value AS real_col,
        CASE 
            WHEN obj.key ~ '^[0-9]+$' THEN obj.value
            ELSE obj.key
        END AS col_alias
    FROM parsed_tables pt
    CROSS JOIN LATERAL jsonb_each_text(
        CASE WHEN jsonb_typeof(pt.col_data) = 'object' THEN pt.col_data ELSE '{{}}'::jsonb END
    ) obj
    
    UNION ALL

    SELECT 
        pt.table_name,
        pt.table_alias,
        arr.value AS real_col,
        arr.value AS col_alias
    FROM parsed_tables pt
    CROSS JOIN LATERAL jsonb_array_elements_text(
        CASE WHEN jsonb_typeof(pt.col_data) = 'array' THEN pt.col_data ELSE '[]'::jsonb END
    ) arr
),
joined_schema AS (
    SELECT 
        pc.table_alias,
        CASE 
            WHEN pc.real_col = '*' THEN COALESCE(c.column_name, 'TABLE_NOT_FOUND') 
            ELSE pc.col_alias 
        END AS final_col_alias,
        CASE 
            WHEN pc.real_col ~* 'THEN\s+(true|false)' THEN 'boolean'
            WHEN pc.real_col ~* '::boolean' THEN 'boolean'
            WHEN pc.real_col ~* '::(integer|int|bigint|smallint)' THEN 'integer'
            WHEN pc.real_col ~* '::(numeric|decimal|real|double)' THEN 'numeric'
            WHEN pc.real_col ~* '^(COUNT|SUM|AVG|MIN|MAX)\(' THEN 'numeric'
            WHEN pc.real_col ~* '[\(\s]|CASE|WHEN|END' THEN 'text' 
            ELSE COALESCE(c.data_type, 'COLUMN_NOT_FOUND_IN_DB') 
        END AS data_type
    FROM parsed_columns pc
    LEFT JOIN information_schema.columns c 
      ON c.table_name = pc.table_name 
      AND c.table_schema = 'public'
      AND (pc.real_col = '*' OR c.column_name = pc.real_col)
),
tables_json AS (
    SELECT jsonb_object_agg(table_alias, cols) AS tables_obj
    FROM (
        SELECT table_alias, jsonb_object_agg(final_col_alias, data_type) AS cols
        FROM joined_schema
        GROUP BY table_alias
    ) subquery
)
SELECT jsonb_build_object(
    {}
) AS result;"#, wl_escaped, build_args_str)
    } else {
        format!(r#"SELECT jsonb_build_object(
    {}
) AS result;"#, build_args_str)
    };

    info_result.insert("sql".to_string(), json!(sql_query));

    let mut result = json!({
        "isOk": true,
        "sql": null,
        "params": null,
        "structure": info_result.clone(),
        "message": "info"
    });
    
    if let Some(sql_val) = info_result.get("sql") {
        result["sql"] = sql_val.clone();
    }
    
    result
}
