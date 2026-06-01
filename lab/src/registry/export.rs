//! Reconstruction of the full, run-specific schedule export artifact.
//!
//! The `schedules` table stores only the *invariant* schedule body (the raw
//! problem annotated with placements). The run-specific `schedule_metadata` and
//! `schedule_metrics` live on the `runs` row. This module recombines them at
//! export time so that each run exports its own metadata and metrics even when
//! many runs share a single deduplicated schedule body.

use serde_json::Value;

use super::row::RunRow;

/// Rebuilds the complete schedule export artifact for `row`.
///
/// The output is semantically identical to what the runner would have produced
/// for that run: the invariant body with this run's `schedule_metadata` and
/// `schedule_metrics` injected as top-level fields.
///
/// Fails loudly (with the run key in context) when the registry is inconsistent
/// or predates the schedule-deduplication fix.
pub fn reconstruct_artifact(row: &RunRow) -> Result<String, String> {
    let short = |s: &str| -> String { s[..s.len().min(16)].to_string() };

    let Some(schedule_hash) = row.schedule_hash.as_deref() else {
        return Err(format!(
            "run '{}' has no schedule_hash; registry is inconsistent",
            short(&row.run_key)
        ));
    };
    let Some(body) = row.schedule_json.as_deref() else {
        return Err(format!(
            "run '{}' references missing schedule '{}'; registry is inconsistent",
            short(&row.run_key),
            short(schedule_hash)
        ));
    };

    let mut value: Value = serde_json::from_str(body).map_err(|e| {
        format!(
            "run '{}' has an invalid stored schedule body: {e}",
            short(&row.run_key)
        )
    })?;
    let obj = value.as_object_mut().ok_or_else(|| {
        format!(
            "run '{}' stored schedule body is not a JSON object",
            short(&row.run_key)
        )
    })?;

    // Defensive: never trust run-specific fields embedded in a shared body.
    obj.remove("schedule_metadata");
    obj.remove("schedule_metrics");

    let Some(metadata_json) = row.metadata_json.as_deref() else {
        return Err(format!(
            "run '{}' has no stored schedule metadata; this database predates the \
             schedule-deduplication fix. Re-run with `lab run --override` or run the \
             one-off migration (lab/src/bin/migrate_schedule_dedup.rs)",
            short(&row.run_key)
        ));
    };
    let metadata: Value = serde_json::from_str(metadata_json).map_err(|e| {
        format!(
            "run '{}' has invalid stored metadata JSON: {e}",
            short(&row.run_key)
        )
    })?;
    obj.insert("schedule_metadata".to_string(), metadata);

    let metrics: Value = serde_json::from_str(&row.metrics_json).map_err(|e| {
        format!(
            "run '{}' has invalid stored metrics JSON: {e}",
            short(&row.run_key)
        )
    })?;
    if metrics.is_object() {
        obj.insert("schedule_metrics".to_string(), metrics);
    }

    serde_json::to_string_pretty(&value)
        .map_err(|e| format!("failed to serialize reconstructed schedule artifact: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use schedulers::schedule::{
        LocationMeta, PeriodMeta, Schedule, ScheduleMetadata, ScheduleOutput, TaskPlacement,
    };
    use schedulers::time::{MJD, TaskId, Time};

    fn sample_schedule() -> Schedule {
        let mut schedule = Schedule::new();
        schedule.insert_placement(TaskPlacement {
            task_id: TaskId(101),
            start: Time::<MJD>::new(62000.1),
            end: Time::<MJD>::new(62000.2),
        });
        schedule
    }

    fn raw_problem() -> serde_json::Value {
        serde_json::json!({
            "scheduling_blocks": [
                { "id": 1, "tasks": [ { "id": 101, "name": "t" } ] }
            ]
        })
    }

    fn metadata(k_beams: u64) -> ScheduleMetadata {
        ScheduleMetadata {
            algorithm: "est".to_string(),
            algorithm_config: serde_json::json!({ "k_beams": k_beams }),
            location: Some(LocationMeta {
                name: "site".to_string(),
                longitude_deg: 1.0,
                latitude_deg: 2.0,
                height_m: 3.0,
            }),
            period: Some(PeriodMeta {
                start_mjd_utc: 62000.0,
                end_mjd_utc: 62001.0,
            }),
            dataset_id: Some("ds1".to_string()),
            dataset_label: None,
        }
    }

    fn row_with(metadata_json: &str, metrics_json: &str, body: &str) -> RunRow {
        RunRow {
            run_key: "a".repeat(64),
            dataset_id: "ds1".to_string(),
            dataset_path: "/d.json".to_string(),
            algorithm: "est".to_string(),
            config_slug: "e1-k1-b1".to_string(),
            identity_json: "{}".to_string(),
            metrics_json: metrics_json.to_string(),
            metadata_json: Some(metadata_json.to_string()),
            schedule_hash: Some("hash".to_string()),
            schedule_json: Some(body.to_string()),
            created_at: "now".to_string(),
            last_seen_at: "now".to_string(),
            source_cell_id: None,
        }
    }

    /// The body-only output recombined with metadata + metrics must equal the
    /// full `ScheduleOutput` serialization (key order aside).
    #[test]
    fn reconstruction_matches_full_schedule_output() {
        let schedule = sample_schedule();
        let md = metadata(1);
        let metrics = serde_json::json!({ "scheduler_runtime_ms": 42.0, "utilization": 0.5 });

        let full = ScheduleOutput::new(raw_problem(), &schedule, Some(md.clone()))
            .with_metrics(metrics.clone());
        let full_json: Value =
            serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();

        let body =
            serde_json::to_string(&ScheduleOutput::new(raw_problem(), &schedule, None)).unwrap();
        let metadata_json = serde_json::to_string(&md).unwrap();
        let metrics_json = serde_json::to_string(&metrics).unwrap();
        let row = row_with(&metadata_json, &metrics_json, &body);

        let rebuilt: Value = serde_json::from_str(&reconstruct_artifact(&row).unwrap()).unwrap();
        assert_eq!(rebuilt, full_json);
    }

    /// Two runs that share one invariant body export their own metadata/metrics.
    #[test]
    fn shared_body_yields_per_run_metadata_and_metrics() {
        let schedule = sample_schedule();
        let body =
            serde_json::to_string(&ScheduleOutput::new(raw_problem(), &schedule, None)).unwrap();

        let md1 = serde_json::to_string(&metadata(1)).unwrap();
        let md2 = serde_json::to_string(&metadata(2)).unwrap();
        let metrics1 = r#"{"scheduler_runtime_ms":10.0}"#;
        let metrics2 = r#"{"scheduler_runtime_ms":20.0}"#;

        let r1 = row_with(&md1, metrics1, &body);
        let r2 = row_with(&md2, metrics2, &body);

        let a: Value = serde_json::from_str(&reconstruct_artifact(&r1).unwrap()).unwrap();
        let b: Value = serde_json::from_str(&reconstruct_artifact(&r2).unwrap()).unwrap();

        assert_eq!(a["schedule_metadata"]["algorithm_config"]["k_beams"], 1);
        assert_eq!(b["schedule_metadata"]["algorithm_config"]["k_beams"], 2);
        assert_eq!(a["schedule_metrics"]["scheduler_runtime_ms"], 10.0);
        assert_eq!(b["schedule_metrics"]["scheduler_runtime_ms"], 20.0);
    }

    /// A stale `schedule_metadata` embedded in the shared body must never leak
    /// into the exported artifact.
    #[test]
    fn embedded_body_metadata_is_overridden() {
        let schedule = sample_schedule();
        // Build a body that wrongly embeds another run's metadata/metrics.
        let poisoned = ScheduleOutput::new(raw_problem(), &schedule, Some(metadata(99)))
            .with_metrics(serde_json::json!({ "scheduler_runtime_ms": 999.0 }));
        let body = serde_json::to_string(&poisoned).unwrap();

        let md = serde_json::to_string(&metadata(7)).unwrap();
        let row = row_with(&md, r#"{"scheduler_runtime_ms":7.0}"#, &body);

        let out: Value = serde_json::from_str(&reconstruct_artifact(&row).unwrap()).unwrap();
        assert_eq!(out["schedule_metadata"]["algorithm_config"]["k_beams"], 7);
        assert_eq!(out["schedule_metrics"]["scheduler_runtime_ms"], 7.0);
    }

    #[test]
    fn missing_metadata_fails_loudly() {
        let mut row = row_with("{}", "{}", r#"{"scheduling_blocks":[]}"#);
        row.metadata_json = None;
        let err = reconstruct_artifact(&row).unwrap_err();
        assert!(err.contains("predates"), "unexpected error: {err}");
    }
}
