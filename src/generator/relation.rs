use std::collections::{HashMap, HashSet, VecDeque};
use super::SqlGenerator;

impl SqlGenerator {
    /// Resolves a relation template from the relations map.
    /// `parent_alias`/`child_alias` — used for KEY lookup (relation keys use aliases).
    /// `parent_real`/`child_real` — used for TEMPLATE replacement (@1, @2, @table → real names in SQL).
    /// Supports `->` (directional), `<->` (bi-directional) keys.
    /// Supports `:node_name` suffix for disambiguating same-table relations.
    pub(crate) fn resolve_relation(
        &self,
        parent_alias: &str, child_alias: &str,
        _parent_real: &str, child_real: &str,
        node_name: &str,
    ) -> Result<Option<String>, String> {
        let symbols = vec!["-><-", "<->", "->", "<-"];
        
        for sym in &symbols {
            let spec = format!("{1}{0}{2}:{3}", sym, parent_alias, child_alias, node_name);
            let key = format!("{1}{0}{2}", sym, parent_alias, child_alias);
            
            // Check specific first, then generic
            for lookup in &[&spec, &key] {
                if let Some(r) = self.relations.get(*lookup) {
                    Self::validate_relation_template(r, parent_alias, child_alias, lookup)?;
                    
                    let table_expr = if child_real == child_alias {
                        child_real.to_string()
                    } else {
                        format!("{} AS {}", child_real, child_alias)
                    };

                    let default_join = match *sym {
                        "-><-" => "INNER JOIN",
                        "<->" => "FULL JOIN",
                        "->" => "LEFT JOIN",
                        "<-" => "RIGHT JOIN",
                        _ => "LEFT JOIN",
                    };

                    let resolved = r
                        .replace("@join", default_join)
                        .replace("@table", &table_expr)
                        .replace("@1", parent_alias)
                        .replace("@2", child_alias);
                    
                    return Ok(Some(resolved));
                }
            }

            // Check reversed for bi-directional symbols
            if *sym == "<->" || *sym == "-><-" {
                let spec_rev = format!("{1}{0}{2}:{3}", sym, child_alias, parent_alias, node_name);
                let key_rev = format!("{1}{0}{2}", sym, child_alias, parent_alias);

                for lookup in &[&spec_rev, &key_rev] {
                    if let Some(r) = self.relations.get(*lookup) {
                        Self::validate_relation_template(r, child_alias, parent_alias, lookup)?;

                        let table_expr = if child_real == child_alias {
                            child_real.to_string()
                        } else {
                            format!("{} AS {}", child_real, child_alias)
                        };

                        let default_join = match *sym {
                            "-><-" => "INNER JOIN",
                            "<->" => "FULL JOIN",
                            _ => "LEFT JOIN",
                        };

                        // a1 is child, a2 is parent because template is reversed
                        let resolved = r
                            .replace("@join", default_join)
                            .replace("@table", &table_expr)
                            .replace("@1", child_alias)
                            .replace("@2", parent_alias);
                        
                        return Ok(Some(resolved));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Builds an adjacency graph from relation keys for path discovery.
    pub(crate) fn build_relation_graph(relations: &HashMap<String, String>) -> HashMap<String, Vec<String>> {
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        let symbols = vec!["-><-", "<->", "->", "<-"];

        for key in relations.keys() {
            for sym in &symbols {
                if let Some(pos) = key.find(sym) {
                    let left = key[..pos].trim();
                    let right_raw = key[pos + sym.len()..].trim();
                    let right = right_raw.split(':').next().unwrap_or(right_raw);

                    if !left.is_empty() && !right.is_empty() {
                        graph.entry(left.to_string()).or_default().push(right.to_string());
                        // Bi-directional symbols
                        if *sym == "<->" || *sym == "-><-" {
                            graph.entry(right.to_string()).or_default().push(left.to_string());
                        }
                        break; // Found the primary symbol
                    }
                }
            }
        }

        for neighbors in graph.values_mut() {
            neighbors.sort();
            neighbors.dedup();
        }

        graph
    }

    /// BFS to find shortest path from `from` to `to` through the cached relation graph.
    pub(crate) fn find_relation_path(&self, from: &str, to: &str) -> Option<Vec<String>> {
        let graph = &self.relation_graph;
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, Vec<String>)> = VecDeque::new();

        visited.insert(from.to_string());
        queue.push_back((from.to_string(), vec![from.to_string()]));

        while let Some((current, path)) = queue.pop_front() {
            if current == to {
                return Some(path);
            }

            if let Some(neighbors) = graph.get(&current) {
                for next in neighbors {
                    if !visited.contains(next) {
                        visited.insert(next.clone());
                        let mut new_path = path.clone();
                        new_path.push(next.clone());
                        queue.push_back((next.clone(), new_path));
                    }
                }
            }
        }

        None
    }

    /// Auto-join all intermediate tables along a discovered path.
    pub(crate) fn auto_join_path(&mut self, path: &[String], override_jt: Option<&str>) -> Result<(), String> {
        for i in 0..path.len() - 1 {
            let step_from = &path[i];
            let step_to = &path[i + 1];

            if self.joined_aliases.contains(step_to) {
                continue;
            }

            let from_real = self.guard.resolve_alias(step_from)?;
            let to_real = self.guard.resolve_alias(step_to)?;
            self.guard.validate_table(step_to)?;

            let mut join = self.resolve_relation(step_from, step_to, &from_real, &to_real, step_to)?
                .ok_or_else(|| format!("No relation template for path step {}->{}", step_from, step_to))?;

            if let Some(jt) = override_jt {
                join = Self::override_join_type(&join, jt);
            }

            self.joins.push(join);
            self.joined_aliases.insert(step_to.clone());
        }

        Ok(())
    }

    /// Overrides the JOIN type keyword in a generated JOIN string.
    pub(crate) fn override_join_type(join_str: &str, join_type: &str) -> String {
        let replacement = match join_type.to_lowercase().as_str() {
            "left" | "->" => "LEFT JOIN",
            "right" | "<-" => "RIGHT JOIN",
            "inner" | "-><-" => "INNER JOIN",
            "full" | "<->" => "FULL JOIN",
            "cross" => "CROSS JOIN",
            _ => return join_str.to_string(),
        };

        let upper = join_str.to_uppercase();
        let keywords = vec!["LEFT JOIN", "RIGHT JOIN", "INNER JOIN", "FULL JOIN", "CROSS JOIN", "JOIN"];
        
        for kw in keywords {
            if upper.starts_with(kw) {
                return format!("{}{}", replacement, &join_str[kw.len()..]);
            }
        }
        
        join_str.to_string()
    }

    /// Only @1, @2, @table placeholders are allowed.
    pub(crate) fn validate_relation_template(template: &str, table1: &str, table2: &str, key: &str) -> Result<(), String> {
        let cleaned = template
            .replace("@join", "")
            .replace("@table", "")
            .replace("@1", "")
            .replace("@2", "");
        
        if cleaned.contains(&format!("{}.", table1)) || cleaned.contains(&format!("{}.", table2)) {
            return Err(format!(
                "Relations Error [{}]: Raw table names used directly. Use @1, @2, or @table placeholders.",
                key
            ));
        }
        
        Ok(())
    }
}

pub(crate) fn extract_on_condition(join_str: &str) -> Result<String, String> {
    let upper = join_str.to_uppercase();
    if let Some(pos) = upper.find(" ON ") {
        Ok(join_str[pos + 4..].trim().to_string())
    } else {
        Err(format!("Cannot extract ON condition from: {}", join_str))
    }
}
