//! SQLite-backed run registry / cache for the lab experiment runner.
//!
//! The registry stores objective/descriptive metrics of every successfully
//! completed scheduler run indexed by a stable content-hash key (`run_key`).
//! On subsequent runs
//! against the same (dataset content, algorithm, config, horizon, versions)
//! the runner can skip re-execution and return the cached metrics instead.
//! Query-time commands decide how to sort, rank, or compare rows; the registry
//! does not persist a subjective "best" decision.
//!
//! # Default location
//! `.lab/runs.sqlite` relative to the current working directory.
//! Override with `--run-db <PATH>`.
//!
//! # Schema version
//! `PRAGMA user_version = 1`.

use chrono::Utc;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::path::Path;

use super::identity::RunIdentity;
use super::query::{BestOpts, DEFAULT_RUN_DB, ListOpts, default_sort_keys, metric_col, sort_expr};
use super::row::{RunRow, row_to_run_row};

// ── Registry ──────────────────────────────────────────────────────────────────

/// Handle to an open run registry database.
pub struct Registry {
    conn: Connection,
}

impl Registry {
    /// Opens (or creates) the registry database at `path`.
    ///
    /// Creates all parent directories as needed.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "failed to create registry directory {}: {e}",
                    parent.display()
                )
            })?;
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| format!("failed to open registry {}: {e}", path.display()))?;
        let mut reg = Self { conn };
        reg.init()?;
        Ok(reg)
    }

    /// Opens the registry at the default path (`.lab/runs.sqlite`).
    pub fn open_default() -> Result<Self, String> {
        Self::open(Path::new(DEFAULT_RUN_DB))
    }

    /// Initialises schema (idempotent).
    fn init(&mut self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "
PRAGMA journal_mode = WAL;
PRAGMA user_version = 1;

CREATE TABLE IF NOT EXISTS runs (
    run_key         TEXT PRIMARY KEY,
    dataset_id      TEXT NOT NULL,
    dataset_path    TEXT NOT NULL,
    dataset_hash    TEXT NOT NULL,
    algorithm       TEXT NOT NULL,
    config_slug     TEXT NOT NULL,
    config_json     TEXT NOT NULL,
    horizon_json    TEXT,
    scheduler_version TEXT NOT NULL,
    metrics_version TEXT NOT NULL,
    identity_json   TEXT NOT NULL,
    metrics_json    TEXT NOT NULL,
    -- indexed metric columns
    task_ratio              REAL,
    priority_ratio          REAL,
    priority_density        REAL,
    utilization             REAL,
    fragmentation_index     REAL,
    runtime_ms              REAL,
    requested_time_sec      REAL,
    scheduled_time_sec      REAL,
    scheduled_time_ratio    REAL,
    -- timestamps
    created_at      TEXT NOT NULL,
    last_seen_at    TEXT NOT NULL,
    source_cell_id  TEXT
);

CREATE INDEX IF NOT EXISTS idx_runs_dataset   ON runs (dataset_id);
CREATE INDEX IF NOT EXISTS idx_runs_algorithm ON runs (algorithm);
CREATE INDEX IF NOT EXISTS idx_runs_config    ON runs (config_slug);
CREATE INDEX IF NOT EXISTS idx_runs_priority_ratio ON runs (priority_ratio DESC);
CREATE INDEX IF NOT EXISTS idx_runs_task_ratio ON runs (task_ratio DESC);
CREATE INDEX IF NOT EXISTS idx_runs_utilization ON runs (utilization DESC);
CREATE INDEX IF NOT EXISTS idx_runs_fragmentation ON runs (fragmentation_index ASC);
CREATE INDEX IF NOT EXISTS idx_runs_runtime ON runs (runtime_ms ASC);
CREATE INDEX IF NOT EXISTS idx_runs_scheduled_time_ratio ON runs (scheduled_time_ratio DESC);
",
            )
            .map_err(|e| format!("failed to init registry schema: {e}"))?;

        // Column-level migrations: add columns introduced after the initial
        // schema so that existing databases are upgraded transparently.
        for (col, ty) in &[
            ("requested_time_sec", "REAL"),
            ("scheduled_time_sec", "REAL"),
            ("scheduled_time_ratio", "REAL"),
            ("schedule_json", "TEXT"),
        ] {
            self.ensure_column("runs", col, ty)?;
        }

        Ok(())
    }

    // ── Write ─────────────────────────────────────────────────────────────────

    /// Adds `col` of type `ty` to `table` if it does not already exist.
    /// Used for incremental schema migrations.
    fn ensure_column(&self, table: &str, col: &str, ty: &str) -> Result<(), String> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info(?1) WHERE name = ?2",
                params![table, col],
                |row| row.get(0),
            )
            .map_err(|e| format!("failed to check column {col} in {table}: {e}"))?;
        if !exists {
            self.conn
                .execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {ty};"))
                .map_err(|e| format!("failed to add column {col} to {table}: {e}"))?;
        }
        Ok(())
    }

    /// Inserts or updates a successful run record.
    ///
    /// On conflict (same `run_key`) the `last_seen_at` timestamp and the
    /// stored metrics are refreshed.  If `schedule_json` is `Some`, the
    /// stored schedule is also updated.
    pub fn upsert(
        &self,
        identity: &RunIdentity,
        metrics_json: &str,
        schedule_json: Option<&str>,
        source_cell_id: Option<&str>,
    ) -> Result<(), String> {
        let run_key = identity.run_key();
        let now = Utc::now().to_rfc3339();
        let identity_json = serde_json::to_string(identity)
            .map_err(|e| format!("failed to serialize identity: {e}"))?;

        // Extract indexed metrics from the JSON blob.
        let mv: serde_json::Value = serde_json::from_str(metrics_json)
            .map_err(|e| format!("failed to parse metrics JSON: {e}"))?;
        let task_ratio = mv["scheduled_task_ratio"].as_f64();
        let priority_ratio = mv["scheduled_priority_ratio"].as_f64();
        let priority_density = mv["priority_density"].as_f64();
        let utilization = mv["utilization"].as_f64();
        let fragmentation_index = mv["fragmentation"]["fragmentation_index"].as_f64();
        let runtime_ms = mv["scheduler_runtime_ms"].as_f64();
        let requested_time_sec = mv["requested_time_sec"].as_f64();
        let scheduled_time_sec = mv["scheduled_time_sec"].as_f64();
        let scheduled_time_ratio = mv["scheduled_time_ratio"].as_f64();

        self.conn
            .execute(
                "INSERT INTO runs (
                    run_key, dataset_id, dataset_path, dataset_hash,
                    algorithm, config_slug, config_json, horizon_json,
                    scheduler_version, metrics_version,
                    identity_json, metrics_json, schedule_json,
                    task_ratio, priority_ratio, priority_density,
                    utilization, fragmentation_index, runtime_ms,
                    requested_time_sec, scheduled_time_sec, scheduled_time_ratio,
                    created_at, last_seen_at, source_cell_id
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                    ?20, ?21, ?22, ?23, ?24, ?25
                )
                ON CONFLICT(run_key) DO UPDATE SET
                    metrics_json          = excluded.metrics_json,
                    schedule_json         = COALESCE(excluded.schedule_json, runs.schedule_json),
                    task_ratio            = excluded.task_ratio,
                    priority_ratio        = excluded.priority_ratio,
                    priority_density      = excluded.priority_density,
                    utilization           = excluded.utilization,
                    fragmentation_index   = excluded.fragmentation_index,
                    runtime_ms            = excluded.runtime_ms,
                    requested_time_sec    = excluded.requested_time_sec,
                    scheduled_time_sec    = excluded.scheduled_time_sec,
                    scheduled_time_ratio  = excluded.scheduled_time_ratio,
                    last_seen_at          = excluded.last_seen_at",
                params![
                    run_key,
                    identity.dataset_id,
                    identity.dataset_path,
                    identity.dataset_hash,
                    identity.algorithm,
                    identity.config_slug,
                    identity.config_json,
                    identity.horizon_json,
                    identity.scheduler_version,
                    identity.metrics_version,
                    identity_json,
                    metrics_json,
                    schedule_json,
                    task_ratio,
                    priority_ratio,
                    priority_density,
                    utilization,
                    fragmentation_index,
                    runtime_ms,
                    requested_time_sec,
                    scheduled_time_sec,
                    scheduled_time_ratio,
                    now,
                    now,
                    source_cell_id,
                ],
            )
            .map_err(|e| format!("failed to upsert registry record: {e}"))?;
        Ok(())
    }

    // ── Read ──────────────────────────────────────────────────────────────────

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
                        identity_json, metrics_json, schedule_json, created_at, last_seen_at, source_cell_id
                   FROM runs WHERE run_key = ?1",
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
        // Build WHERE clause and params vector dynamically.
        let mut conditions: Vec<String> = Vec::new();
        let mut param_idx: usize = 1;

        // We collect owned SQL values to pass as parameters.
        // These are held in variables so references remain valid.
        let dataset_val = if opts.dataset.is_some() {
            conditions.push(format!("dataset_id = ?{param_idx}"));
            param_idx += 1;
            opts.dataset.clone()
        } else {
            None
        };

        let algorithm_val = if opts.algorithm.is_some() {
            conditions.push(format!("algorithm = ?{param_idx}"));
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
        // suppress unused warning when both metric filters are absent
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
                    identity_json, metrics_json, schedule_json, created_at, last_seen_at, source_cell_id
               FROM runs {where_clause} ORDER BY {order} LIMIT {limit}"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| format!("registry list query failed: {e}"))?;

        // Collect all present params in order as &dyn ToSql references.
        // The lifetimes work because all the Option<T> values live until
        // end of this function.
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
            ("WHERE dataset_id = ?1 AND algorithm = ?2", true)
        } else {
            ("WHERE dataset_id = ?1", false)
        };

        let sql = format!(
            "SELECT run_key, dataset_id, dataset_path, algorithm, config_slug,
                    identity_json, metrics_json, schedule_json, created_at, last_seen_at, source_cell_id
               FROM runs {where_clause} ORDER BY {order} LIMIT {limit}"
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
