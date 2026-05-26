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
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Metrics version tag included in the identity hash.
/// Increment (e.g. `"schedule_metrics/2"`) whenever the metric surface changes
/// in a way that makes old cached values incompatible.
pub const METRICS_VERSION: &str = "schedule_metrics/1";

/// Default path of the SQLite registry file, relative to the working dir.
pub const DEFAULT_RUN_DB: &str = ".lab/runs.sqlite";

// ── Version helpers ───────────────────────────────────────────────────────────

/// Scheduler version string: `$GIT_SHA` (injected at build time via
/// `build.rs`) when present, otherwise `"lab/<version>+schedulers/<version>"`.
pub fn scheduler_version() -> String {
    // Set by build.rs if available.
    if let Some(sha) = option_env!("GIT_SHA")
        && !sha.is_empty()
    {
        return sha.to_string();
    }
    let lab_ver = env!("CARGO_PKG_VERSION");
    // schedulers crate version is not directly accessible from lab, so we use
    // what we have.
    format!("lab/{lab_ver}")
}

// ── Identity ──────────────────────────────────────────────────────────────────

/// The semantic identity of one scheduler run.
///
/// Two runs with the same `RunIdentity` are considered deterministically
/// equivalent; only one needs to be executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunIdentity {
    /// Dataset ID string (from the experiment spec).
    pub dataset_id: String,
    /// Absolute or canonical path of the dataset file (for provenance /
    /// regeneration only — NOT part of the hash).
    pub dataset_path: String,
    /// SHA-256 hex of the raw dataset file bytes.
    pub dataset_hash: String,
    /// Algorithm name (`"est"` or `"hap"`).
    pub algorithm: String,
    /// Configuration slug (e.g. `"e1-k4-b2"` or `"hap-i128-r3-p4-elitist4-s0"`).
    pub config_slug: String,
    /// Full configuration as a compact JSON string.
    pub config_json: String,
    /// Horizon override serialised as compact JSON, or `null`.
    pub horizon_json: Option<String>,
    /// Scheduler/lab version string.
    pub scheduler_version: String,
    /// Metrics schema version.
    pub metrics_version: String,
}

/// The subset of fields that contribute to the `run_key` hash.
///
/// `dataset_path` is intentionally excluded so the same dataset content
/// is treated as the same run regardless of where it lives on disk.
#[derive(Serialize)]
struct RunIdentityHashable<'a> {
    dataset_id: &'a str,
    dataset_hash: &'a str,
    algorithm: &'a str,
    config_slug: &'a str,
    config_json: &'a str,
    horizon_json: Option<&'a str>,
    scheduler_version: &'a str,
    metrics_version: &'a str,
}

impl RunIdentity {
    /// Computes the stable `run_key`: SHA-256 of the canonical compact JSON of
    /// the semantic-identity fields (path excluded).
    pub fn run_key(&self) -> String {
        let hashable = RunIdentityHashable {
            dataset_id: &self.dataset_id,
            dataset_hash: &self.dataset_hash,
            algorithm: &self.algorithm,
            config_slug: &self.config_slug,
            config_json: &self.config_json,
            horizon_json: self.horizon_json.as_deref(),
            scheduler_version: &self.scheduler_version,
            metrics_version: &self.metrics_version,
        };
        // Sort keys for stability; serde_json with BTreeMap ordering is
        // ensured by serializing a struct (field order is declaration order).
        let canonical = serde_json::to_string(&hashable)
            .expect("RunIdentityHashable serialization is infallible");
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{digest:x}")
    }
}

// ── Dataset hash ──────────────────────────────────────────────────────────────

/// Computes the SHA-256 hex digest of the file at `path`.
pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("failed to read dataset for hashing {}: {e}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}

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

// ── Query option types ────────────────────────────────────────────────────────

/// Options for [`Registry::list`].
#[derive(Debug, Default)]
pub struct ListOpts {
    pub dataset: Option<String>,
    pub algorithm: Option<String>,
    pub metric: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub sort: Vec<SortKey>,
    pub limit: Option<usize>,
}

/// Options for [`Registry::best`].
#[derive(Debug)]
pub struct BestOpts {
    pub dataset_id: String,
    pub algorithm: Option<String>,
    pub sort: Vec<SortKey>,
    pub limit: Option<usize>,
}

/// Direction for a query-time sort key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    pub const fn as_sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

/// One metric/direction pair used for query-time ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub metric: String,
    pub direction: SortDirection,
}

/// Parse `metric:asc` / `metric:desc`. If direction is omitted, the metric's
/// conventional objective direction is used.
pub fn parse_sort_key(input: &str) -> Result<SortKey, String> {
    let (metric, dir) = input
        .split_once(':')
        .map_or((input, None), |(m, d)| (m, Some(d)));
    let metric = metric.trim();
    if metric.is_empty() {
        return Err("sort key metric cannot be empty".to_string());
    }
    let direction = match dir.map(str::trim) {
        Some("asc") => SortDirection::Asc,
        Some("desc") => SortDirection::Desc,
        Some(other) => {
            return Err(format!(
                "invalid sort direction '{other}' in '{input}', expected asc or desc"
            ));
        }
        None => default_metric_direction(metric)?,
    };
    metric_col(metric)?;
    Ok(SortKey {
        metric: metric.to_string(),
        direction,
    })
}

/// Default query-time policy used only when the user does not provide sort
/// keys. It is deliberately expressed as objective metric ordering, not as a
/// persisted composite score.
pub fn default_sort_keys() -> Vec<SortKey> {
    vec![
        SortKey {
            metric: "scheduled_priority_ratio".to_string(),
            direction: SortDirection::Desc,
        },
        SortKey {
            metric: "scheduled_task_ratio".to_string(),
            direction: SortDirection::Desc,
        },
        SortKey {
            metric: "priority_density".to_string(),
            direction: SortDirection::Desc,
        },
        SortKey {
            metric: "runtime_ms".to_string(),
            direction: SortDirection::Asc,
        },
    ]
}

// ── Row type ──────────────────────────────────────────────────────────────────

/// A registry row returned by query helpers.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub run_key: String,
    pub dataset_id: String,
    pub dataset_path: String,
    pub algorithm: String,
    pub config_slug: String,
    pub identity_json: String,
    pub metrics_json: String,
    pub schedule_json: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
    pub source_cell_id: Option<String>,
}

fn row_to_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        run_key: row.get(0)?,
        dataset_id: row.get(1)?,
        dataset_path: row.get(2)?,
        algorithm: row.get(3)?,
        config_slug: row.get(4)?,
        identity_json: row.get(5)?,
        metrics_json: row.get(6)?,
        schedule_json: row.get(7)?,
        created_at: row.get(8)?,
        last_seen_at: row.get(9)?,
        source_cell_id: row.get(10)?,
    })
}

// ── Metric helpers ────────────────────────────────────────────────────────────

pub fn metric_col(metric: &str) -> Result<&'static str, String> {
    match metric {
        "task_ratio" | "scheduled_task_ratio" => Ok("task_ratio"),
        "priority_ratio" | "scheduled_priority_ratio" => Ok("priority_ratio"),
        "priority_density" => Ok("priority_density"),
        "utilization" => Ok("utilization"),
        "fragmentation_index" => Ok("fragmentation_index"),
        "runtime_ms" | "scheduler_runtime_ms" => Ok("runtime_ms"),
        "requested_time_sec" => Ok("requested_time_sec"),
        "scheduled_time_sec" => Ok("scheduled_time_sec"),
        "scheduled_time_ratio" => Ok("scheduled_time_ratio"),
        "composite_score" | "composite_rank_score" => Err(
            "composite_rank_score is not a registry sort/filter metric; use `registry rank --weight ...` to compute a query-time score"
                .to_string(),
        ),
        _ => Err(format!("unsupported registry metric '{metric}'")),
    }
}

fn default_metric_direction(metric: &str) -> Result<SortDirection, String> {
    Ok(match metric_col(metric)? {
        "fragmentation_index" | "runtime_ms" => SortDirection::Asc,
        _ => SortDirection::Desc,
    })
}

fn sort_expr(keys: &[SortKey]) -> Result<String, String> {
    let mut parts = Vec::with_capacity(keys.len() + 1);
    for key in keys {
        let col = metric_col(&key.metric)?;
        parts.push(format!("{col} {}", key.direction.as_sql()));
    }
    parts.push("run_key ASC".to_string());
    Ok(parts.join(", "))
}

/// Returns the registry path from an optional CLI override.
pub fn registry_path(run_db: Option<&Path>) -> PathBuf {
    run_db
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_RUN_DB))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_identity(dataset_hash: &str, config_json: &str) -> RunIdentity {
        RunIdentity {
            dataset_id: "ds1".to_string(),
            dataset_path: "/data/ds1.json".to_string(),
            dataset_hash: dataset_hash.to_string(),
            algorithm: "est".to_string(),
            config_slug: "e1-k1-b1".to_string(),
            config_json: config_json.to_string(),
            horizon_json: None,
            scheduler_version: "v1".to_string(),
            metrics_version: METRICS_VERSION.to_string(),
        }
    }

    #[test]
    fn same_inputs_produce_same_run_key() {
        let id = make_identity("aabbcc", r#"{"k_beams":1}"#);
        assert_eq!(id.run_key(), id.run_key());
    }

    #[test]
    fn different_dataset_hash_changes_run_key() {
        let id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let id2 = make_identity("hash2", r#"{"k_beams":1}"#);
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn different_config_changes_run_key() {
        let id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let id2 = make_identity("hash1", r#"{"k_beams":2}"#);
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn different_horizon_changes_run_key() {
        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let mut id2 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.horizon_json = None;
        id2.horizon_json = Some(r#"{"start_mjd":62000,"end_mjd":62001}"#.to_string());
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn different_scheduler_version_changes_run_key() {
        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let mut id2 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.scheduler_version = "v1".to_string();
        id2.scheduler_version = "v2".to_string();
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn different_metrics_version_changes_run_key() {
        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let mut id2 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.metrics_version = "schedule_metrics/1".to_string();
        id2.metrics_version = "schedule_metrics/2".to_string();
        assert_ne!(id1.run_key(), id2.run_key());
    }

    #[test]
    fn dataset_path_is_not_part_of_run_key() {
        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        let mut id2 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.dataset_path = "/path/a/ds1.json".to_string();
        id2.dataset_path = "/path/b/ds1.json".to_string();
        assert_eq!(id1.run_key(), id2.run_key());
    }

    fn open_temp_registry() -> (Registry, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.sqlite");
        let reg = Registry::open(&path).unwrap();
        (reg, dir)
    }

    const SAMPLE_METRICS: &str = r#"{
        "scheduled_task_ratio": 0.8,
        "scheduled_priority_ratio": 0.9,
        "priority_density": 1.1,
        "utilization": 0.75,
        "fragmentation": {"fragmentation_index": 0.2, "gap_count": 2, "gap_total_sec": 100.0, "largest_gap_sec": 60.0},
        "composite_rank_score": 0.85,
        "scheduler_runtime_ms": 42.0
    }"#;

    #[test]
    fn schema_initialization_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.sqlite");
        let _ = Registry::open(&path).unwrap();
        let _ = Registry::open(&path).unwrap(); // second open should succeed
    }

    #[test]
    fn insert_and_lookup_by_key() {
        let (reg, _dir) = open_temp_registry();
        let id = make_identity("deadbeef", r#"{"k_beams":1}"#);
        let key = id.run_key();

        assert!(!reg.contains(&key).unwrap());
        reg.upsert(&id, SAMPLE_METRICS, None, Some("ds1__est__e1-k1-b1"))
            .unwrap();
        assert!(reg.contains(&key).unwrap());

        let row = reg.get_row(&key).unwrap().unwrap();
        assert_eq!(row.dataset_id, "ds1");
        assert_eq!(row.algorithm, "est");
    }

    #[test]
    fn upsert_refreshes_existing_record() {
        let (reg, _dir) = open_temp_registry();
        let id = make_identity("deadbeef", r#"{"k_beams":1}"#);
        reg.upsert(&id, SAMPLE_METRICS, None, None).unwrap();
        // Second upsert should not error.
        reg.upsert(&id, SAMPLE_METRICS, None, None).unwrap();
        // Only one row should exist.
        let rows = reg
            .list(&ListOpts {
                dataset: Some("ds1".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn list_filters_by_dataset() {
        let (reg, _dir) = open_temp_registry();

        let mut id1 = make_identity("hash1", r#"{"k_beams":1}"#);
        id1.dataset_id = "ds1".to_string();
        id1.config_slug = "e1-k1-b1".to_string();
        let mut id2 = make_identity("hash2", r#"{"k_beams":2}"#);
        id2.dataset_id = "ds2".to_string();
        id2.config_slug = "e1-k2-b1".to_string();

        reg.upsert(&id1, SAMPLE_METRICS, None, None).unwrap();
        reg.upsert(&id2, SAMPLE_METRICS, None, None).unwrap();

        let rows = reg
            .list(&ListOpts {
                dataset: Some("ds1".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].dataset_id, "ds1");
    }

    #[test]
    fn best_returns_ordered_by_metric() {
        let (reg, _dir) = open_temp_registry();

        let metrics_a = r#"{"scheduled_task_ratio":0.6,"scheduled_priority_ratio":0.6,"priority_density":1.0,"utilization":0.5,"fragmentation":{"fragmentation_index":0.3,"gap_count":1,"gap_total_sec":50.0,"largest_gap_sec":50.0},"composite_rank_score":0.5,"scheduler_runtime_ms":10.0}"#;
        let metrics_b = r#"{"scheduled_task_ratio":0.9,"scheduled_priority_ratio":0.95,"priority_density":1.05,"utilization":0.85,"fragmentation":{"fragmentation_index":0.1,"gap_count":0,"gap_total_sec":0.0,"largest_gap_sec":0.0},"composite_rank_score":0.9,"scheduler_runtime_ms":20.0}"#;

        let mut id_a = make_identity("hash1", r#"{"k_beams":1}"#);
        id_a.config_slug = "e1-k1-b1".to_string();
        let mut id_b = make_identity("hash1", r#"{"k_beams":2}"#);
        id_b.config_slug = "e1-k2-b1".to_string();

        reg.upsert(&id_a, metrics_a, None, None).unwrap();
        reg.upsert(&id_b, metrics_b, None, None).unwrap();

        let rows = reg
            .best(&BestOpts {
                dataset_id: "ds1".to_string(),
                algorithm: None,
                sort: vec![parse_sort_key("scheduled_priority_ratio:desc").unwrap()],
                limit: Some(2),
            })
            .unwrap();
        assert_eq!(rows.len(), 2);
        // Best scheduled priority ratio (0.95) should come first.
        assert_eq!(rows[0].config_slug, "e1-k2-b1");
    }

    #[test]
    fn composite_score_is_not_a_registry_sort_metric() {
        let err = parse_sort_key("composite_score:desc").unwrap_err();
        assert!(err.contains("query-time score"));
    }

    #[test]
    fn prefix_resolution_succeeds_for_unique_prefix() {
        let (reg, _dir) = open_temp_registry();
        let id = make_identity("deadbeef", r#"{"k_beams":1}"#);
        let key = id.run_key();
        reg.upsert(&id, SAMPLE_METRICS, None, None).unwrap();

        let prefix = &key[..16];
        let resolved = reg.resolve_prefix(prefix).unwrap();
        assert_eq!(resolved, key);
    }

    #[test]
    fn prefix_resolution_errors_on_missing() {
        let (reg, _dir) = open_temp_registry();
        let result = reg.resolve_prefix("nonexistentprefix");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no run found"));
    }
}
