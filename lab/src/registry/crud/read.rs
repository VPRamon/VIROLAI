//! Read operations for the run registry.

use rusqlite::{OptionalExtension, params};

use super::super::query::{BestOpts, ListOpts, default_sort_keys, metric_col, sort_expr};
use super::super::row::{RunRow, row_to_run_row};
use super::super::store::Registry;

impl Registry {
    /// Returns the number of unique schedule payloads stored.
    pub fn schedule_count(&self) -> Result<usize, String> {
        self.conn
            .query_row("SELECT COUNT(*) FROM schedules", [], |row| {
                row.get::<_, usize>(0)
            })
            .map_err(|e| format!("failed to count schedules: {e}"))
    }

    /// Returns the stored schedule JSON for `schedule_hash`, if present.
    pub fn get_schedule_json(&self, schedule_hash: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT schedule_json FROM schedules WHERE schedule_hash = ?1",
                params![schedule_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("failed to get schedule {schedule_hash}: {e}"))
    }

    /// Returns `true` if a record with `run_key` exists.
    pub fn contains(&self, run_key: &str) -> Result<bool, String> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE run_key = ?1",
                params![run_key],
                |row| row.get(0),
            )
            .map_err(|e| format!("registry lookup failed: {e}"))?;
        Ok(count > 0)
    }

    /// Returns the `metrics_json` for a run identified by its full key.
    pub fn get_metrics(&self, run_key: &str) -> Result<Option<String>, String> {
        let result = self
            .conn
            .query_row(
                "SELECT metrics_json FROM runs WHERE run_key = ?1",
                params![run_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("registry get_metrics failed: {e}"))?;
        Ok(result)
    }

    /// Returns the full `RunRow` for a run identified by its full key.
    pub fn get_row(&self, run_key: &str) -> Result<Option<RunRow>, String> {
        let result = self
            .conn
            .query_row(
                "SELECT run_key, dataset_id, dataset_path, algorithm, config_slug,
                        identity_json, metrics_json, r.schedule_hash, s.schedule_json,
                        r.created_at, r.last_seen_at, r.source_cell_id
                   FROM runs r
                   LEFT JOIN schedules s ON r.schedule_hash = s.schedule_hash
                  WHERE run_key = ?1",
                params![run_key],
                row_to_run_row,
            )
            .optional()
            .map_err(|e| format!("registry get_row failed: {e}"))?;
        Ok(result)
    }

    /// Resolves a full run key from a unique prefix.
    ///
    /// Returns an error if no run or more than one run matches.
    pub fn resolve_prefix(&self, prefix: &str) -> Result<String, String> {
        let pattern = format!("{prefix}%");
        let mut stmt = self
            .conn
            .prepare("SELECT run_key FROM runs WHERE run_key LIKE ?1 LIMIT 3")
            .map_err(|e| format!("registry prefix query failed: {e}"))?;
        let keys: Vec<String> = stmt
            .query_map(params![pattern], |row| row.get(0))
            .map_err(|e| format!("registry prefix query failed: {e}"))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("registry prefix query row failed: {e}"))?;
        match keys.len() {
            0 => Err(format!("no run found matching prefix '{prefix}'")),
            1 => Ok(keys.into_iter().next().unwrap()),
            _ => Err(format!(
                "ambiguous prefix '{prefix}' matches {} runs",
                keys.len()
            )),
        }
    }

    /// Lists run records matching optional filters.
    pub fn list(&self, opts: &ListOpts) -> Result<Vec<RunRow>, String> {
        let mut conditions: Vec<String> = Vec::new();
        let mut param_idx: usize = 1;

        let dataset_val = if opts.dataset.is_some() {
            conditions.push(format!("r.dataset_id = ?{param_idx}"));
            param_idx += 1;
            opts.dataset.clone()
        } else {
            None
        };

        let algorithm_val = if opts.algorithm.is_some() {
            conditions.push(format!("r.algorithm = ?{param_idx}"));
            param_idx += 1;
            opts.algorithm.clone()
        } else {
            None
        };

        let (min_val, max_val) = if let Some(metric) = &opts.metric {
            let min = if opts.min.is_some() {
                let col = metric_col(metric)?;
                conditions.push(format!("{col} >= ?{param_idx}"));
                param_idx += 1;
                opts.min
            } else {
                None
            };
            let max = if opts.max.is_some() {
                let col = metric_col(metric)?;
                conditions.push(format!("{col} <= ?{param_idx}"));
                param_idx += 1;
                opts.max
            } else {
                None
            };
            (min, max)
        } else {
            (None, None)
        };
        let _ = param_idx;

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sort = if opts.sort.is_empty() {
            default_sort_keys()
        } else {
            opts.sort.clone()
        };
        let order = sort_expr(&sort)?;
        let limit = opts.limit.unwrap_or(100);

        let sql = format!(
            "SELECT run_key, dataset_id, dataset_path, algorithm, config_slug,
                    identity_json, metrics_json, r.schedule_hash, s.schedule_json,
                    r.created_at, r.last_seen_at, r.source_cell_id
               FROM runs r
               LEFT JOIN schedules s ON r.schedule_hash = s.schedule_hash
               {where_clause} ORDER BY {order} LIMIT {limit}"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("registry list query failed: {e}"))?;

        let mut params: Vec<&dyn rusqlite::types::ToSql> = Vec::new();
        if let Some(ref v) = dataset_val {
            params.push(v);
        }
        if let Some(ref v) = algorithm_val {
            params.push(v);
        }
        if let Some(ref v) = min_val {
            params.push(v);
        }
        if let Some(ref v) = max_val {
            params.push(v);
        }

        let rows = stmt
            .query_map(params.as_slice(), row_to_run_row)
            .map_err(|e| format!("registry list query failed: {e}"))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("registry list row failed: {e}"))?;
        Ok(rows)
    }

    /// Returns the best runs for a dataset ordered by query-time sort keys.
    pub fn best(&self, opts: &BestOpts) -> Result<Vec<RunRow>, String> {
        let sort = if opts.sort.is_empty() {
            default_sort_keys()
        } else {
            opts.sort.clone()
        };
        let order = sort_expr(&sort)?;
        let limit = opts.limit.unwrap_or(10);

        let (where_clause, has_algo) = if opts.algorithm.is_some() {
            ("WHERE r.dataset_id = ?1 AND r.algorithm = ?2", true)
        } else {
            ("WHERE r.dataset_id = ?1", false)
        };

        let sql = format!(
            "SELECT run_key, dataset_id, dataset_path, algorithm, config_slug,
                    identity_json, metrics_json, r.schedule_hash, s.schedule_json,
                    r.created_at, r.last_seen_at, r.source_cell_id
               FROM runs r
               LEFT JOIN schedules s ON r.schedule_hash = s.schedule_hash
               {where_clause} ORDER BY {order} LIMIT {limit}"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("registry best query failed: {e}"))?;

        let rows = if has_algo {
            stmt.query_map(
                params![opts.dataset_id, opts.algorithm.as_deref().unwrap_or("")],
                row_to_run_row,
            )
        } else {
            stmt.query_map(params![opts.dataset_id], row_to_run_row)
        }
        .map_err(|e| format!("registry best query failed: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("registry best row failed: {e}"))?;
        Ok(rows)
    }
}
