//! Create (insert / upsert) operations for the run registry.

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use super::super::identity::RunIdentity;
use super::super::store::Registry;

impl Registry {
    /// Inserts an invariant schedule body if that semantic schedule is not
    /// already stored.
    ///
    /// `schedule_json` must contain only invariant placement data; run-specific
    /// `schedule_metadata` / `schedule_metrics` must **not** be embedded here
    /// because the row is keyed by the shared semantic `schedule_hash`.
    ///
    /// Existing schedule rows are never overwritten. If a row already exists for
    /// `schedule_hash` with a *different* `dataset_hash`, the insertion is
    /// rejected as a hash collision across datasets.
    pub fn upsert_schedule(
        &self,
        schedule_hash: &str,
        dataset_hash: &str,
        schedule_json: &str,
    ) -> Result<(), String> {
        insert_schedule(&self.conn, schedule_hash, dataset_hash, schedule_json)
    }

    /// Inserts the invariant schedule body and inserts or updates the run.
    pub fn upsert_result(
        &mut self,
        identity: &RunIdentity,
        metrics_json: &str,
        metadata_json: &str,
        schedule_hash: &str,
        schedule_json: &str,
        source_cell_id: Option<&str>,
    ) -> Result<(), String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("failed to start registry transaction: {e}"))?;
        insert_schedule(&tx, schedule_hash, &identity.dataset_hash, schedule_json)?;
        upsert_run_record_sql(
            &tx,
            identity,
            metrics_json,
            Some(metadata_json),
            Some(schedule_hash),
            source_cell_id,
        )?;
        tx.commit()
            .map_err(|e| format!("failed to commit registry transaction: {e}"))
    }

    /// Inserts or updates a successful run record without an associated
    /// schedule body.
    ///
    /// On conflict (same `run_key`) the `last_seen_at` timestamp and the stored
    /// metrics are refreshed. If `schedule_hash` is `Some`, the run is linked to
    /// the corresponding row in `schedules`.
    pub fn upsert(
        &mut self,
        identity: &RunIdentity,
        metrics_json: &str,
        schedule_hash: Option<&str>,
        source_cell_id: Option<&str>,
    ) -> Result<(), String> {
        upsert_run_record_sql(
            &self.conn,
            identity,
            metrics_json,
            None,
            schedule_hash,
            source_cell_id,
        )
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Inserts the invariant schedule body, guarding against cross-dataset hash
/// collisions.
///
/// `conn` accepts either a [`Connection`] or a `Transaction` (which derefs to
/// `Connection`).
fn insert_schedule(
    conn: &Connection,
    schedule_hash: &str,
    dataset_hash: &str,
    schedule_json: &str,
) -> Result<(), String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT dataset_hash FROM schedules WHERE schedule_hash = ?1",
            params![schedule_hash],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("failed to look up schedule {schedule_hash}: {e}"))?;

    if let Some(existing_dataset_hash) = existing {
        if existing_dataset_hash != dataset_hash {
            return Err(format!(
                "schedule hash collision: {schedule_hash} already stored for dataset \
                 {existing_dataset_hash} but a run reported dataset {dataset_hash}"
            ));
        }
        // Invariant body already stored; keep the first one.
        return Ok(());
    }

    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO schedules (
            schedule_hash, dataset_hash, schedule_json, created_at
        ) VALUES (?1, ?2, ?3, ?4)",
        params![schedule_hash, dataset_hash, schedule_json, now],
    )
    .map_err(|e| format!("failed to insert schedule {schedule_hash}: {e}"))?;
    Ok(())
}

fn upsert_run_record_sql(
    conn: &Connection,
    identity: &RunIdentity,
    metrics_json: &str,
    metadata_json: Option<&str>,
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

    conn.execute(
        "INSERT INTO runs (
            run_key, dataset_id, dataset_path, dataset_hash,
            algorithm, config_slug, config_json, horizon_json,
            scheduler_version, metrics_version,
            identity_json, metrics_json, metadata_json, schedule_hash,
            task_ratio, priority_ratio, priority_density,
            utilization, fragmentation_index, runtime_ms,
            requested_time_sec, scheduled_time_sec, scheduled_time_ratio,
            created_at, last_seen_at, source_cell_id
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22, ?23, ?24, ?25, ?26
        )
        ON CONFLICT(run_key) DO UPDATE SET
            metrics_json          = excluded.metrics_json,
            metadata_json         = COALESCE(excluded.metadata_json, runs.metadata_json),
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
            metadata_json,
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
