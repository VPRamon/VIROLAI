//! Create (insert / upsert) operations for the run registry.

use chrono::Utc;
use rusqlite::{Connection, Transaction, params};

use super::super::super::identity::RunIdentity;
use super::super::Registry;

impl Registry {
    /// Inserts a schedule payload if that semantic schedule is not already stored.
    ///
    /// Existing schedule rows are never overwritten.
    pub fn upsert_schedule(
        &self,
        schedule_hash: &str,
        dataset_hash: &str,
        schedule_json: &str,
    ) -> Result<(), String> {
        insert_schedule(&self.conn, schedule_hash, dataset_hash, schedule_json)
    }

    /// Inserts a schedule payload and inserts or updates the corresponding run.
    pub fn upsert_result(
        &mut self,
        identity: &RunIdentity,
        metrics_json: &str,
        schedule_hash: &str,
        schedule_json: &str,
        source_cell_id: Option<&str>,
    ) -> Result<(), String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("failed to start registry transaction: {e}"))?;
        insert_schedule(&tx, schedule_hash, &identity.dataset_hash, schedule_json)?;
        upsert_run_record(
            &tx,
            identity,
            metrics_json,
            Some(schedule_hash),
            source_cell_id,
        )?;
        tx.commit()
            .map_err(|e| format!("failed to commit registry transaction: {e}"))
    }

    /// Inserts or updates a successful run record.
    ///
    /// On conflict (same `run_key`) the `last_seen_at` timestamp and the
    /// stored metrics are refreshed. If `schedule_hash` is `Some`, the run is
    /// linked to the corresponding row in `schedules`.
    pub fn upsert(
        &mut self,
        identity: &RunIdentity,
        metrics_json: &str,
        schedule_hash: Option<&str>,
        source_cell_id: Option<&str>,
    ) -> Result<(), String> {
        upsert_run_record(
            &self.conn,
            identity,
            metrics_json,
            schedule_hash,
            source_cell_id,
        )
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

fn insert_schedule(
    conn: &impl ScheduleInserter,
    schedule_hash: &str,
    dataset_hash: &str,
    schedule_json: &str,
) -> Result<(), String> {
    conn.insert_schedule(schedule_hash, dataset_hash, schedule_json)
}

trait ScheduleInserter {
    fn insert_schedule(
        &self,
        schedule_hash: &str,
        dataset_hash: &str,
        schedule_json: &str,
    ) -> Result<(), String>;
}

impl ScheduleInserter for Connection {
    fn insert_schedule(
        &self,
        schedule_hash: &str,
        dataset_hash: &str,
        schedule_json: &str,
    ) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        self.execute(
            "INSERT OR IGNORE INTO schedules (
                schedule_hash, dataset_hash, schedule_json, created_at
            ) VALUES (?1, ?2, ?3, ?4)",
            params![schedule_hash, dataset_hash, schedule_json, now],
        )
        .map_err(|e| format!("failed to upsert schedule {schedule_hash}: {e}"))?;
        Ok(())
    }
}

impl ScheduleInserter for Transaction<'_> {
    fn insert_schedule(
        &self,
        schedule_hash: &str,
        dataset_hash: &str,
        schedule_json: &str,
    ) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        self.execute(
            "INSERT OR IGNORE INTO schedules (
                schedule_hash, dataset_hash, schedule_json, created_at
            ) VALUES (?1, ?2, ?3, ?4)",
            params![schedule_hash, dataset_hash, schedule_json, now],
        )
        .map_err(|e| format!("failed to upsert schedule {schedule_hash}: {e}"))?;
        Ok(())
    }
}

fn upsert_run_record(
    conn: &impl RunUpserter,
    identity: &RunIdentity,
    metrics_json: &str,
    schedule_hash: Option<&str>,
    source_cell_id: Option<&str>,
) -> Result<(), String> {
    conn.upsert_run_record(identity, metrics_json, schedule_hash, source_cell_id)
}

trait RunUpserter {
    fn upsert_run_record(
        &self,
        identity: &RunIdentity,
        metrics_json: &str,
        schedule_hash: Option<&str>,
        source_cell_id: Option<&str>,
    ) -> Result<(), String>;
}

fn upsert_run_record_sql(
    executor: &impl SqlExecutor,
    identity: &RunIdentity,
    metrics_json: &str,
    schedule_hash: Option<&str>,
    source_cell_id: Option<&str>,
) -> Result<(), String> {
    let run_key = identity.run_key();
    let now = Utc::now().to_rfc3339();
    let identity_json = serde_json::to_string(identity)
        .map_err(|e| format!("failed to serialize identity: {e}"))?;

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

    executor
        .execute_sql(
            "INSERT INTO runs (
            run_key, dataset_id, dataset_path, dataset_hash,
            algorithm, config_slug, config_json, horizon_json,
            scheduler_version, metrics_version,
            identity_json, metrics_json, schedule_hash,
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
            schedule_hash         = COALESCE(excluded.schedule_hash, runs.schedule_hash),
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
                schedule_hash,
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

trait SqlExecutor {
    fn execute_sql<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize>;
}

impl SqlExecutor for Connection {
    fn execute_sql<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        self.execute(sql, params)
    }
}

impl SqlExecutor for Transaction<'_> {
    fn execute_sql<P: rusqlite::Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        self.execute(sql, params)
    }
}

impl RunUpserter for Connection {
    fn upsert_run_record(
        &self,
        identity: &RunIdentity,
        metrics_json: &str,
        schedule_hash: Option<&str>,
        source_cell_id: Option<&str>,
    ) -> Result<(), String> {
        upsert_run_record_sql(self, identity, metrics_json, schedule_hash, source_cell_id)
    }
}

impl RunUpserter for Transaction<'_> {
    fn upsert_run_record(
        &self,
        identity: &RunIdentity,
        metrics_json: &str,
        schedule_hash: Option<&str>,
        source_cell_id: Option<&str>,
    ) -> Result<(), String> {
        upsert_run_record_sql(self, identity, metrics_json, schedule_hash, source_cell_id)
    }
}
