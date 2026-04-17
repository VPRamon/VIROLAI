//! Schedule output schema: the original problem JSON annotated with time allocations.
//!
//! [`ScheduleOutput`] carries the raw problem JSON and the computed placements.
//! Serializing it produces a document that mirrors the input format but with
//! `scheduled_start_mjd_utc`, `scheduled_end_mjd_utc`, and `scheduled` fields
//! added to each task object.

use crate::schedule::Schedule;
use crate::time::{MJD, TaskId};
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::collections::HashMap;

/// The scheduling result expressed in the same JSON envelope as the input.
///
/// Each task inside `scheduling_blocks` (or the legacy top-level array) is
/// annotated with three extra fields:
/// - `scheduled`: `true` when the task was placed, `false` otherwise.
/// - `scheduled_start_mjd_utc`: placement start in MJD UTC (only when `scheduled` is `true`).
/// - `scheduled_end_mjd_utc`: placement end in MJD UTC (only when `scheduled` is `true`).
pub struct ScheduleOutput {
    raw_problem: Value,
    placements: HashMap<u64, (f64, f64)>,
}

impl ScheduleOutput {
    /// Build an output from the raw input JSON and the computed schedule.
    pub fn new(raw_problem: Value, schedule: &Schedule) -> Self {
        let placements = schedule
            .placements
            .iter()
            .map(|(TaskId(id), p)| {
                (
                    *id,
                    (
                        p.start.to::<MJD>().value(),
                        p.end.to::<MJD>().value(),
                    ),
                )
            })
            .collect();

        Self {
            raw_problem,
            placements,
        }
    }
}

impl Serialize for ScheduleOutput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut augmented = self.raw_problem.clone();
        annotate_blocks(&mut augmented, &self.placements);
        augmented.serialize(serializer)
    }
}

fn annotate_blocks(json: &mut Value, placements: &HashMap<u64, (f64, f64)>) {
    // Envelope format: { "scheduling_blocks": [...] }
    if let Some(blocks) = json.get_mut("scheduling_blocks").and_then(Value::as_array_mut) {
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
