//! SQLite-backed run registry / cache for the lab experiment runner.
//!
//! The registry stores objective/descriptive metrics of every successfully
//! completed scheduler run indexed by a stable content-hash key (`run_key`).
//! On subsequent runs against the same (dataset content, algorithm, config,
//! horizon, versions) the runner can skip re-execution and return the cached
//! metrics instead.  Query-time commands decide how to sort, rank, or compare
//! rows; the registry does not persist a subjective "best" decision.
//!
//! # Default location
//! `.lab/runs.sqlite` relative to the current working directory.
//! Override with `--run-db <PATH>`.
//!
//! # Schema version
//! `PRAGMA user_version = 1`.

use rusqlite::{Connection, OpenFlags};
use std::path::Path;

use super::query::DEFAULT_RUN_DB;

// ── Registry ──────────────────────────────────────────────────────────────────

/// Handle to an open run registry database.
pub struct Registry {
    /// SQLite connection. Visible to the entire `registry` module tree so that
    /// the `crud` submodule can implement `impl Registry` methods there.
    pub(in crate::registry) conn: Connection,
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
    schedule_hash   TEXT,
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

CREATE TABLE IF NOT EXISTS schedules (
    schedule_hash TEXT PRIMARY KEY,
    dataset_hash  TEXT NOT NULL,
    schedule_json TEXT NOT NULL,
    created_at    TEXT NOT NULL
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
CREATE INDEX IF NOT EXISTS idx_schedules_dataset_hash ON schedules(dataset_hash);
",
            )
            .map_err(|e| format!("failed to init registry schema: {e}"))?;

        // Column-level migrations: add columns introduced after the initial
        // schema so that existing databases are upgraded transparently.
        for (col, ty) in &[
            ("requested_time_sec", "REAL"),
            ("scheduled_time_sec", "REAL"),
            ("scheduled_time_ratio", "REAL"),
            ("schedule_hash", "TEXT"),
        ] {
            self.ensure_column("runs", col, ty)?;
        }
        self.conn
            .execute_batch(
                "
CREATE INDEX IF NOT EXISTS idx_runs_schedule_hash ON runs(schedule_hash);
CREATE INDEX IF NOT EXISTS idx_schedules_dataset_hash ON schedules(dataset_hash);
",
            )
            .map_err(|e| format!("failed to init registry indexes: {e}"))?;

        Ok(())
    }

    /// Adds `col` of type `ty` to `table` if it does not already exist.
    /// Used for incremental schema migrations.
    fn ensure_column(&self, table: &str, col: &str, ty: &str) -> Result<(), String> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info(?1) WHERE name = ?2",
                rusqlite::params![table, col],
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
}
