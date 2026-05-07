//! Local mirror of `state.jsonl` event shapes.
//!
//! The matrix runner lives in a separate binary so we cannot import its
//! types; we redeclare a structurally-identical [`StateEvent`] /
//! [`CellStatus`] pair here (kept in sync with
//! `scripts/experiment_matrix/state.rs`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CellStatus {
    Started,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEvent {
    pub cell_id: String,
    pub status: CellStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub schedule_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metrics_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub finished_at: Option<String>,
}

/// Read every event in `state.jsonl`. Tolerates blank lines; missing file
/// returns an empty vector.
pub fn read_events(path: &Path) -> std::io::Result<Vec<StateEvent>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for (lineno, line) in reader.lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<StateEvent>(trimmed) {
            Ok(ev) => events.push(ev),
            Err(e) => {
                tracing::warn!(
                    "skipping malformed state event in {} line {}: {e}",
                    path.display(),
                    lineno + 1
                );
            }
        }
    }
    Ok(events)
}

/// Latest event per cell, keyed by `cell_id`.
pub fn latest_per_cell(events: &[StateEvent]) -> HashMap<String, &StateEvent> {
    let mut out: HashMap<String, &StateEvent> = HashMap::new();
    for ev in events {
        out.insert(ev.cell_id.clone(), ev);
    }
    out
}

/// Aggregate counts: (started_only, completed, failed).
pub fn count_statuses(events: &[StateEvent]) -> (usize, usize, usize) {
    let latest = latest_per_cell(events);
    let mut started = 0;
    let mut completed = 0;
    let mut failed = 0;
    for ev in latest.values() {
        match ev.status {
            CellStatus::Started => started += 1,
            CellStatus::Completed => completed += 1,
            CellStatus::Failed => failed += 1,
        }
    }
    (started, completed, failed)
}
