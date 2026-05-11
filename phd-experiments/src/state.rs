//! Append-only checkpoint stream (`state.jsonl`).
//!
//! Each cell execution appends two [`StateEvent`] records to `state.jsonl`:
//! one with status [`CellStatus::Started`] at the beginning, and one with
//! [`CellStatus::Completed`] or [`CellStatus::Failed`] at the end.
//!
//! The file is written through a [`StateWriter`] that holds a mutex-protected
//! buffered handle, making it safe to call from multiple Rayon threads
//! simultaneously.
//!
//! On resume, [`read_events`] replays the log and [`completed_cells`] extracts
//! the set of cells whose *latest* event is `completed`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ── State event types ─────────────────────────────────────────────────────────

/// One line in `state.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEvent {
    /// Identifier of the cell this event refers to.
    pub cell_id: String,
    /// Current execution status.
    pub status: CellStatus,
    /// Path to the schedule JSON, present when `status` is `completed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule_path: Option<String>,
    /// Path to the metrics JSON, present when `status` is `completed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics_path: Option<String>,
    /// Error message, present when `status` is `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// RFC 3339 timestamp recorded when the cell started.
    pub started_at: String,
    /// RFC 3339 timestamp recorded when the cell finished (absent for `started`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

/// Execution status of a single matrix cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellStatus {
    /// Cell has started but not yet finished.
    Started,
    /// Cell completed successfully.
    Completed,
    /// Cell terminated with an error.
    Failed,
}

// ── State writer ──────────────────────────────────────────────────────────────

/// Thread-safe append-only writer for `state.jsonl`.
pub struct StateWriter {
    inner: Mutex<BufWriter<File>>,
    path: PathBuf,
}

impl StateWriter {
    /// Opens (or creates) the state file in append mode.
    pub fn open_append(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("failed to open state stream {}: {e}", path.display()))?;
        Ok(Self {
            inner: Mutex::new(BufWriter::new(file)),
            path: path.to_path_buf(),
        })
    }

    /// Serialises `event` as a single JSON line and flushes the buffer.
    pub fn append(&self, event: &StateEvent) -> Result<(), String> {
        let line = serde_json::to_string(event)
            .map_err(|e| format!("failed to serialize state event: {e}"))?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "state writer mutex poisoned".to_string())?;
        writeln!(*guard, "{line}").map_err(|e| {
            format!(
                "failed to write state event to {}: {e}",
                self.path.display()
            )
        })?;
        guard
            .flush()
            .map_err(|e| format!("failed to flush state stream {}: {e}", self.path.display()))
    }
}

// ── Reader helpers ────────────────────────────────────────────────────────────

/// Reads every event from `state.jsonl`.
///
/// Tolerates blank lines; returns an empty vector if the file does not exist.
pub fn read_events(path: &Path) -> Result<Vec<StateEvent>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)
        .map_err(|e| format!("failed to open state stream {}: {e}", path.display()))?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| {
            format!(
                "failed to read state stream {} line {}: {e}",
                path.display(),
                lineno + 1
            )
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event: StateEvent = serde_json::from_str(trimmed).map_err(|e| {
            format!(
                "malformed state event in {} line {}: {e}",
                path.display(),
                lineno + 1
            )
        })?;
        events.push(event);
    }
    Ok(events)
}

/// Returns the set of cell IDs whose *latest* event has status `completed`.
pub fn completed_cells(events: &[StateEvent]) -> std::collections::HashSet<String> {
    let mut latest: HashMap<String, CellStatus> = HashMap::new();
    for ev in events {
        latest.insert(ev.cell_id.clone(), ev.status);
    }
    latest
        .into_iter()
        .filter(|(_, s)| *s == CellStatus::Completed)
        .map(|(k, _)| k)
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_and_read_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.jsonl");
        let writer = StateWriter::open_append(&path).unwrap();
        writer
            .append(&StateEvent {
                cell_id: "a".into(),
                status: CellStatus::Started,
                schedule_path: None,
                metrics_path: None,
                error: None,
                started_at: "t0".into(),
                finished_at: None,
            })
            .unwrap();
        writer
            .append(&StateEvent {
                cell_id: "a".into(),
                status: CellStatus::Completed,
                schedule_path: Some("schedules/a.json".into()),
                metrics_path: Some("metrics/a.json".into()),
                error: None,
                started_at: "t0".into(),
                finished_at: Some("t1".into()),
            })
            .unwrap();
        drop(writer);

        let events = read_events(&path).unwrap();
        assert_eq!(events.len(), 2);
        let done = completed_cells(&events);
        assert!(done.contains("a"));
    }

    #[test]
    fn read_returns_empty_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("absent.jsonl");
        assert!(read_events(&path).unwrap().is_empty());
    }

    #[test]
    fn completed_uses_latest_event() {
        let events = vec![
            StateEvent {
                cell_id: "x".into(),
                status: CellStatus::Completed,
                schedule_path: None,
                metrics_path: None,
                error: None,
                started_at: "t0".into(),
                finished_at: Some("t1".into()),
            },
            StateEvent {
                cell_id: "x".into(),
                status: CellStatus::Failed,
                schedule_path: None,
                metrics_path: None,
                error: Some("boom".into()),
                started_at: "t2".into(),
                finished_at: Some("t3".into()),
            },
        ];
        let done = completed_cells(&events);
        assert!(!done.contains("x"));
    }
}
