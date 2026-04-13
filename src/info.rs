use indexmap::IndexMap;
use serde_json::{json, Value};
use std::collections::HashMap;

struct TableMapping {
    real_name: String,
    col_map: HashMap<String, String>,
    unrestricted_star: bool,
}

pub fn process_info_request(
    info_arr: &[Value],
    whitelist_str: Option<&str>,
    relations_str: Option<&str>,
    macros_str: Option<&str>,
) -> Value {
    let is_tables = info_arr.iter().any(|v| v.as_str() == Some("@tables"));
    let is_relations = info_arr.iter().any(|v| v.as_str() == Some("@relations"));
    
    let mut info_result = serde_json::Map::new();
    let mut included_relations = String::from("[]");

    let macros: IndexMap<String, Value> = macros_str.and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();

    if is_relations && relations_str.is_some() {
        let rel_str = relations_str.unwrap_or("{}");
        let rel_map: IndexMap<String, String> = serde_json::from_str(rel_str).unwrap_or_default();
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
        let wl: IndexMap<String, Value> = serde_json::from_str(wl_str).unwrap_or_default();
        
        // Build table alias mapping from whitelist
        let mut table_mappings = HashMap::new();
        for (key, val) in &wl {
            let (real, alias) = if let Some((n, a)) = key.split_once(':') {
                (n, a)
            } else {
                (key.as_str(), key.as_str())
            };
            
            let mut col_map = HashMap::new();
            let mut unrestricted_star = false;
            
            match val {
                Value::Object(obj) => {
                    for (k, v) in obj {
                        if let Some(v_str) = v.as_str() {
                            col_map.insert(k.clone(), v_str.to_string());
                        }
                    }
                }
                Value::Array(arr) => {
                    for v in arr {
                        if v.as_str() == Some("*") { unrestricted_star = true; break; }
                        if let Some(s) = v.as_str() { col_map.insert(s.to_string(), s.to_string()); }
                    }
                }
                Value::String(s) if s == "*" => {
                    unrestricted_star = true;
                }
                _ => {
                    unrestricted_star = true; // Fallback for complex structures or empty
                }
            }
            
            table_mappings.insert(alias.to_string(), TableMapping {
                real_name: real.to_string(),
                col_map,
                unrestricted_star,
            });
        }

        // Resolve macros and normal tables in whitelist
        let mut resolved_wl = IndexMap::new();
        for (key, val) in wl {
            let (_, alias) = if let Some((n, a)) = key.split_once(':') {
                (n, a)
            } else {
                (key.as_str(), key.as_str())
            };
            
            let mut final_mappings = IndexMap::new();
            if let Some(macro_val) = macros.get(alias).or_else(|| {
                // Also check by the "name" part (before :)
                if let Some((n, _)) = key.split_once(':') {
                    macros.get(n)
                } else {
                    None
                }
            }) {
                // It's a macro - Collect all exposed fields with their real table:col mapping
                final_mappings = collect_macro_fields(macro_val, &table_mappings);
                
                // Merge with whitelist's own fields for this macro alias
                let current_source = macro_val.get("@source").and_then(|v| v.as_str());
                let base_table_alias = current_source.map(|s| crate::parser::parse_source(s).table_name).unwrap_or_else(|| alias.to_string());
                
                if let Some(tm) = table_mappings.get(&base_table_alias) {
                    process_fields(&val, &tm, &mut final_mappings, false); // Don't allow '*' from whitelist to force everything if macro says otherwise
                } else {
                    let tm_dummy = TableMapping { real_name: base_table_alias, col_map: HashMap::new(), unrestricted_star: true };
                    process_fields(&val, &tm_dummy, &mut final_mappings, false);
                }
            } else {
                // Normal table
                if let Some(tm) = table_mappings.get(alias) {
                    process_fields(&val, tm, &mut final_mappings, true);
                }
            }
            resolved_wl.insert(alias.to_string(), json!(final_mappings));
        }

        let wl_final_str = serde_json::to_string(&resolved_wl).unwrap();
        let wl_escaped = wl_final_str.replace("'", "''");

        format!(r#"WITH input_json AS (
    SELECT '{}'::jsonb AS data
),
parsed_tables AS (
    SELECT 
        key AS table_alias,
        value AS col_data
    FROM input_json, jsonb_each(data)
),
parsed_columns AS (
    SELECT 
        pt.table_alias,
        split_part(obj.value, ':', 1) AS table_name,
        substr(obj.value, length(split_part(obj.value, ':', 1)) + 2) AS real_col,
        obj.key AS col_alias
    FROM parsed_tables pt
    CROSS JOIN LATERAL jsonb_each_text(pt.col_data) obj
),
joined_schema AS (
    SELECT 
        pc.table_alias,
        CASE 
            WHEN pc.real_col = '*' THEN c.column_name 
            ELSE pc.col_alias 
        END AS final_col_alias,
        CASE 
            WHEN pc.real_col ~* 'THEN\s+(true|false)' THEN 'boolean'
            WHEN pc.real_col ~* '::boolean' THEN 'boolean'
            WHEN pc.real_col ~* '::(integer|int|bigint|smallint)' THEN 'integer'
            WHEN pc.real_col ~* '::(numeric|decimal|real|double)' THEN 'numeric'
            WHEN pc.real_col ~* '^(COUNT|SUM|AVG|MIN|MAX)\(' THEN 'numeric'
            WHEN pc.real_col ~* '[\(\s]|CASE|WHEN|END' THEN 'expression' 
            ELSE COALESCE(c.data_type, 'virtual') 
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
        FROM (
            SELECT DISTINCT table_alias, final_col_alias, data_type 
            FROM joined_schema 
            WHERE final_col_alias IS NOT NULL
        ) uniq
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
        "message": "info"
    });
    
    if let Some(sql_val) = info_result.get("sql") {
        result["sql"] = sql_val.clone();
    }
    if let Some(rels_val) = info_result.get("relations") {
        result["relations"] = rels_val.clone();
    }
    
    result
}

fn process_fields(val: &Value, tm: &TableMapping, mappings: &mut IndexMap<String, String>, allow_star: bool) {
    match val {
        Value::Object(obj) => {
            for (fk, fv) in obj {
                if let Some(fv_str) = fv.as_str() {
                    let real_col = tm.col_map.get(fv_str).map(|s| s.as_str()).unwrap_or(fv_str);
                    mappings.insert(fk.clone(), format!("{}:{}", tm.real_name, real_col));
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                if let Some(s) = v.as_str() { 
                    if s == "*" {
                        if allow_star { 
                            if tm.unrestricted_star {
                                mappings.insert("*".to_string(), format!("{}:*", tm.real_name)); 
                            } else {
                                // Expand * using the col_map from whitelist
                                for (fk, fv) in &tm.col_map {
                                    mappings.insert(fk.clone(), format!("{}:{}", tm.real_name, fv));
                                }
                            }
                        }
                    } else {
                        let real_col = tm.col_map.get(s).map(|s| s.as_str()).unwrap_or(s);
                        mappings.insert(s.to_string(), format!("{}:{}", tm.real_name, real_col)); 
                    }
                }
            }
        }
        Value::String(s) if s == "*" => {
            if allow_star { 
                if tm.unrestricted_star {
                    mappings.insert("*".to_string(), format!("{}:*", tm.real_name)); 
                } else {
                    for (fk, fv) in &tm.col_map {
                        mappings.insert(fk.clone(), format!("{}:{}", tm.real_name, fv));
                    }
                }
            }
        }
        _ => {
            if allow_star && mappings.is_empty() {
                if tm.unrestricted_star {
                    mappings.insert("*".to_string(), format!("{}:*", tm.real_name));
                } else {
                    for (fk, fv) in &tm.col_map {
                        mappings.insert(fk.clone(), format!("{}:{}", tm.real_name, fv));
                    }
                }
            }
        }
    }
}

fn collect_macro_fields(
    macro_val: &Value, 
    table_mappings: &HashMap<String, TableMapping>
) -> IndexMap<String, String> {
    let mut fields = IndexMap::new();
    
    let current_source = macro_val.get("@source").and_then(|v| v.as_str());
    let current_table_alias = current_source.map(|s| crate::parser::parse_source(s).table_name);
    
    if let Some(alias) = &current_table_alias {
        if let Some(tm) = table_mappings.get(alias) {
            if let Some(f) = macro_val.get("@fields") {
                process_fields(f, tm, &mut fields, true);
            }
        } else {
            if let Some(f) = macro_val.get("@fields") {
                 let dummy = TableMapping { real_name: alias.clone(), col_map: HashMap::new(), unrestricted_star: true };
                 process_fields(f, &dummy, &mut fields, true);
            }
        }
    }

    if let Some(obj) = macro_val.as_object() {
        // Children
        for (k, v) in obj {
            if !k.starts_with('@') {
                if let Some(child_obj) = v.as_object() {
                    let is_flatten = child_obj.get("@flatten").and_then(|v| v.as_bool()).unwrap_or(false);
                    if is_flatten {
                        let child_fields = collect_macro_fields(v, table_mappings);
                        for (fk, fv) in child_fields {
                            fields.insert(fk, fv);
                        }
                    }
                }
            }
        }
    }
    fields
}
