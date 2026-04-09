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
        // Specific keys (with :node_name suffix)
        let spec_dir = format!("{}->{}:{}", parent_alias, child_alias, node_name);
        let spec_bi1 = format!("{}<->{}:{}", parent_alias, child_alias, node_name);
        let spec_bi2 = format!("{}<->{}:{}", child_alias, parent_alias, node_name);
        // Generic keys
        let key_dir = format!("{}->{}", parent_alias, child_alias);
        let key_bi1 = format!("{}<->{}", parent_alias, child_alias);
        let key_bi2 = format!("{}<->{}", child_alias, parent_alias);

        let candidates: Vec<(&str, bool)> = vec![
            (&spec_dir, false),
            (&spec_bi1, false),
            (&spec_bi2, true),
            (&key_dir, false),
            (&key_bi1, false),
            (&key_bi2, true),
        ];

        for (key, reversed) in candidates {
            if let Some(r) = self.relations.get(key) {
                // Validate: template must use @1, @2, @table — not raw alias names
                let (t1, t2) = if reversed { (child_alias, parent_alias) } else { (parent_alias, child_alias) };
                Self::validate_relation_template(r, t1, t2, key)?;
                // Replace: @table → "real AS alias", @1/@2 → alias (SQL alias)
                let (a1, a2) = if reversed {
                    (child_alias, parent_alias)
                } else {
                    (parent_alias, child_alias)
                };
                let child_a = if reversed { child_alias } else { child_alias };
                let child_r = if reversed { child_real } else { child_real };
                let table_expr = if child_r == child_a {
                    child_r.to_string() // No alias needed (no whitelist alias)
                } else {
                    format!("{} AS {}", child_r, child_a)
                };
                let resolved = r
                    .replace("@table", &table_expr)
                    .replace("@1", a1)
                    .replace("@2", a2);
                return Ok(Some(resolved));
            }
        }

        Ok(None)
    }

    /// Builds an adjacency graph from relation keys for path discovery.
    /// Parses keys like "a->b", "a<->b", "a->b:name" into edges.
    pub(crate) fn build_relation_graph(relations: &HashMap<String, String>) -> HashMap<String, Vec<String>> {
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();

        for key in relations.keys() {
            let is_bidi = key.contains("<->");
            let parts: Vec<&str> = if is_bidi {
                key.splitn(2, "<->").collect()
            } else {
                key.splitn(2, "->").collect()
            };

            if parts.len() != 2 { continue; }

            let left = parts[0].trim();
            // Strip :node_name suffix from right side
            let right_raw = parts[1].trim();
            let right = right_raw.split(':').next().unwrap_or(right_raw);

            graph.entry(left.to_string()).or_default().push(right.to_string());
            if is_bidi {
                graph.entry(right.to_string()).or_default().push(left.to_string());
            }
        }

        // Deduplicate neighbors
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
    /// Returns the JOINs for all steps from path[0] to path[last].
    pub(crate) fn auto_join_path(&mut self, path: &[String]) -> Result<(), String> {
        for i in 0..path.len() - 1 {
            let step_from = &path[i];
            let step_to = &path[i + 1];

            // Skip if this intermediate table is already joined
            if i > 0 && self.joined_aliases.contains(step_from) {
                // Already joined, check next step
            }
            if self.joined_aliases.contains(step_to) {
                continue; // Target already joined
            }

            let from_real = self.guard.resolve_alias(step_from)?;
            let to_real = self.guard.resolve_alias(step_to)?;
            self.guard.validate_table(step_to)?;

            let join = self.resolve_relation(step_from, step_to, &from_real, &to_real, step_to)?;

            if let Some(j) = join {
                if !j.to_uppercase().contains("JOIN") {
                    return Err(format!("Invalid JOIN syntax in path {}->{}", step_from, step_to));
                }
                self.joins.push(j);
                self.joined_aliases.insert(step_to.clone());
            } else {
                return Err(format!("No relation template for path step {}->{}", step_from, step_to));
            }
        }

        Ok(())
    }

    /// Overrides the JOIN type keyword in a generated JOIN string.
    /// e.g., "INNER JOIN table ON ..." → "LEFT JOIN table ON ..."
    pub(crate) fn override_join_type(join_str: &str, join_type: &str) -> String {
        let upper = join_str.to_uppercase();
        let replacement = match join_type.as_ref() {
            "left" => "LEFT JOIN",
            "right" => "RIGHT JOIN",
            "inner" => "INNER JOIN",
            "cross" => "CROSS JOIN",
            _ => return join_str.to_string(), // unknown → no change
        };
        // Replace any existing JOIN type prefix
        if upper.starts_with("INNER JOIN") {
            format!("{}{}", replacement, &join_str[10..])
        } else if upper.starts_with("LEFT JOIN") {
            format!("{}{}", replacement, &join_str[9..])
        } else if upper.starts_with("RIGHT JOIN") {
            format!("{}{}", replacement, &join_str[10..])
        } else if upper.starts_with("CROSS JOIN") {
            format!("{}{}", replacement, &join_str[10..])
        } else if upper.starts_with("JOIN") {
            format!("{}{}", replacement, &join_str[4..])
        } else {
            join_str.to_string()
        }
    }

    /// Only @1, @2, @table placeholders are allowed.
    pub(crate) fn validate_relation_template(template: &str, table1: &str, table2: &str, key: &str) -> Result<(), String> {
        // Remove all valid placeholders first, then check for raw names
        let cleaned = template
            .replace("@table", "")
            .replace("@1", "")
            .replace("@2", "");
        
        // Check if any raw table name still appears (with dot after it = column reference)
        if cleaned.contains(&format!("{}.", table1)) {
            return Err(format!(
                "Relations Error [{}]: Raw table name '{}.' used directly. Use @1, @2, or @table placeholders instead.",
                key, table1
            ));
        }
        if cleaned.contains(&format!("{}.", table2)) {
            return Err(format!(
                "Relations Error [{}]: Raw table name '{}.' used directly. Use @1, @2, or @table placeholders instead.",
                key, table2
            ));
        }
        
        // Also check JOIN target (after JOIN keyword, before ON)
        let upper = template.to_uppercase();
        if let Some(join_pos) = upper.find("JOIN ") {
            let after_join = &template[join_pos + 5..];
            let target = after_join.split_whitespace().next().unwrap_or("");
            if target != "@table" && target != "@1" && target != "@2" {
                return Err(format!(
                    "Relations Error [{}]: Raw table name '{}' used after JOIN. Use @table, @1, or @2 placeholder instead.",
                    key, target
                ));
            }
        }
        
        Ok(())
    }
}

/// Extracts the ON condition from a JOIN string.
/// E.g. "INNER JOIN foo ON bar.id = foo.bar_id AND foo.status = 1"
/// => "bar.id = foo.bar_id AND foo.status = 1"
pub(crate) fn extract_on_condition(join_str: &str) -> Result<String, String> {
    let upper = join_str.to_uppercase();
    if let Some(pos) = upper.find(" ON ") {
        Ok(join_str[pos + 4..].trim().to_string())
    } else {
        Err(format!("Cannot extract ON condition from: {}", join_str))
    }
}
