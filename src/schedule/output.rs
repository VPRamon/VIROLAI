//! Schedule output schema: the original problem JSON annotated with time allocations.
//!
//! [`ScheduleOutput`] carries the raw problem JSON and the computed placements.
//! Serializing it produces a document that mirrors the input format but with
//! `scheduled_start_mjd_utc`, `scheduled_end_mjd_utc`, and `scheduled` fields
//! added to each task object, plus an optional top-level `schedule_metadata`
//! field containing the algorithm configuration, observing site, and horizon.

use crate::schedule::Schedule;
use crate::time::{MJD, TaskId};
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::collections::HashMap;

// ── Schedule metadata types ───────────────────────────────────────────────────

/// Observatory location embedded in the schedule metadata.
#[derive(Debug, Clone, Serialize)]
pub struct LocationMeta {
    /// Human-readable telescope name.
    pub name: String,
    /// Geodetic longitude in degrees (east positive).
    pub longitude_deg: f64,
    /// Geodetic latitude in degrees.
    pub latitude_deg: f64,
    /// Ellipsoidal height above WGS84 in metres.
    pub height_m: f64,
}

/// Scheduling horizon embedded in the schedule metadata.
#[derive(Debug, Clone, Serialize)]
pub struct PeriodMeta {
    /// Inclusive UTC start in Modified Julian Date.
    pub start_mjd_utc: f64,
    /// Exclusive UTC end in Modified Julian Date.
    pub end_mjd_utc: f64,
}

/// Top-level metadata recorded alongside every schedule output.
///
/// Serialized as `schedule_metadata` in the exported JSON so that downstream
/// tools (e.g. the TSI webapp) can identify the algorithm and configuration
/// without parsing the file name.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleMetadata {
    /// Algorithm identifier: `"est"` or `"hap"`.
    pub algorithm: String,
    /// Algorithm-specific configuration parameters (free-form JSON object).
    pub algorithm_config: Value,
    /// Observing site used for this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<LocationMeta>,
    /// Scheduling horizon used for this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<PeriodMeta>,
}

// ── ScheduleOutput ────────────────────────────────────────────────────────────

/// The scheduling result expressed in the same JSON envelope as the input.
///
/// Each task inside `scheduling_blocks` (or the legacy top-level array) is
/// annotated with three extra fields:
/// - `scheduled`: `true` when the task was placed, `false` otherwise.
/// - `scheduled_start_mjd_utc`: placement start in MJD UTC (only when `scheduled` is `true`).
/// - `scheduled_end_mjd_utc`: placement end in MJD UTC (only when `scheduled` is `true`).
///
/// When `metadata` is supplied it is serialized as a top-level
/// `schedule_metadata` object.
pub struct ScheduleOutput {
    raw_problem: Value,
    placements: HashMap<u64, (f64, f64)>,
    metadata: Option<ScheduleMetadata>,
}

impl ScheduleOutput {
    /// Build an output from the raw input JSON, the computed schedule, and
    /// optional run metadata.
    pub fn new(
        raw_problem: Value,
        schedule: &Schedule,
        metadata: Option<ScheduleMetadata>,
    ) -> Self {
        let placements = schedule
            .placements
            .iter()
            .map(|(TaskId(id), p)| {
                (
                    *id,
                    (p.start.to::<MJD>().value(), p.end.to::<MJD>().value()),
                )
            })
            .collect();

        Self {
            raw_problem,
            placements,
            metadata,
        }
    }
}

impl Serialize for ScheduleOutput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut augmented = self.raw_problem.clone();
        annotate_blocks(&mut augmented, &self.placements);
        if let Some(ref metadata) = self.metadata
            && let Some(obj) = augmented.as_object_mut()
        {
            let meta_value = serde_json::to_value(metadata).map_err(serde::ser::Error::custom)?;
            obj.insert("schedule_metadata".to_string(), meta_value);
        }
        augmented.serialize(serializer)
    }
}

fn annotate_blocks(json: &mut Value, placements: &HashMap<u64, (f64, f64)>) {
    // Envelope format: { "scheduling_blocks": [...] }
    if let Some(blocks) = json
        .get_mut("scheduling_blocks")
        .and_then(Value::as_array_mut)
    {
        for block in blocks {
            annotate_tasks_in_block(block, placements);
        }
    // Legacy format: top-level array of blocks
    } else if let Some(blocks) = json.as_array_mut() {
        for block in blocks {
            annotate_tasks_in_block(block, placements);
        }
    }
}

fn annotate_tasks_in_block(block: &mut Value, placements: &HashMap<u64, (f64, f64)>) {
    if let Some(tasks) = block.get_mut("tasks").and_then(Value::as_array_mut) {
        for task in tasks {
            let Some(id) = task.get("id").and_then(Value::as_u64) else {
                continue;
            };
            if let Some((start, end)) = placements.get(&id) {
                task["scheduled"] = Value::Bool(true);
                task["scheduled_start_mjd_utc"] = serde_json::json!(start);
                task["scheduled_end_mjd_utc"] = serde_json::json!(end);
            } else {
                task["scheduled"] = Value::Bool(false);
            }
        }
    }
}
