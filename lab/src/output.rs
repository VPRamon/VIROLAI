//! Output directory layout for experiment runs.
//!
//! An experiment run is stored under:
//!
//! ```text
//! <output_dir>/<experiment_slug>/run-<timestamp>/
//!   schedules/          — one self-contained schedule JSON per cell
//!                         (includes `schedule_metadata` and the embedded
//!                         `schedule_metrics` block)
//!   experiment.json     — resolved spec + full cell list
//!   state.jsonl         — append-only checkpoint stream (omitted when
//!                         the runner is invoked with `--no-state`)
//! ```
//!
//! Note: there is no separate `metrics/` directory and no `traces/`
//! directory. Metrics live inside each schedule. Traces are not part of
//! the canonical layout — if reintroduced they would be referenced by a
//! manifest as a workspace-stored artifact.
//!
//! This module provides helpers to create that layout, compute per-cell
//! file paths, and serialise / deserialise the `experiment.json`
//! manifest.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::cell::MatrixCell;
use crate::spec::ExperimentSpec;

/// Subdirectory for schedule JSON files.
pub const SCHEDULES_DIR: &str = "schedules";
/// Append-only checkpoint stream filename.
pub const STATE_FILE: &str = "state.jsonl";
/// Resolved spec + cell-list manifest filename.
pub const EXPERIMENT_FILE: &str = "experiment.json";

// ── Directory creation ────────────────────────────────────────────────────────

/// Converts an arbitrary experiment name into a filesystem-safe directory
/// component by replacing non-alphanumeric (and non-`_`) characters with `-`
/// and trimming leading/trailing dashes.
///
/// Falls back to `"experiment"` for strings that reduce to empty.
pub fn experiment_slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "experiment".to_string()
    } else {
        trimmed
    }
}

/// Creates `<output_dir>/<experiment_slug>/run-<ts>/` together with the
/// standard `schedules/` and `metrics/` subdirectories.
///
/// Appends a numeric suffix if the timestamped name already exists (up to
/// 100 attempts before returning an error).
pub fn create_run_dir(output_dir: &Path, experiment_name: &str) -> Result<PathBuf, String> {
    let exp_dir = output_dir.join(experiment_slug(experiment_name));
    fs::create_dir_all(&exp_dir)
        .map_err(|e| format!("failed to create {}: {e}", exp_dir.display()))?;

    let now = Utc::now();
    let base = format!(
        "run-{}-{:09}Z",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    );
    for suffix in 0..100 {
        let name = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        let dir = exp_dir.join(name);
        match fs::create_dir(&dir) {
            Ok(()) => {
                init_subdirs(&dir)?;
                return Ok(dir);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("failed to create {}: {e}", dir.display())),
        }
    }
    Err(format!(
        "exhausted timestamped names under {}",
        exp_dir.display()
    ))
}

/// Ensures the `schedules/` subdirectory exists inside an existing run directory.
pub fn init_subdirs(run_dir: &Path) -> Result<(), String> {
    let p = run_dir.join(SCHEDULES_DIR);
    fs::create_dir_all(&p).map_err(|e| format!("failed to create {}: {e}", p.display()))?;
    Ok(())
}

// ── Experiment manifest ───────────────────────────────────────────────────────

/// The serialised spec and full cell list written to `experiment.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentManifest {
    /// Original experiment specification.
    pub spec: ExperimentSpec,
    /// Fully resolved list of cells (Cartesian product of datasets × configs).
    pub cells: Vec<MatrixCell>,
}

/// Writes `experiment.json` to `run_dir`.
pub fn write_manifest(
    run_dir: &Path,
    spec: &ExperimentSpec,
    cells: &[MatrixCell],
) -> Result<(), String> {
    let manifest = ExperimentManifest {
        spec: spec.clone(),
        cells: cells.to_vec(),
    };
    let path = run_dir.join(EXPERIMENT_FILE);
    let text = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("failed to serialize experiment manifest: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

/// Reads and deserialises `experiment.json` from `run_dir`.
#[allow(dead_code)]
pub fn read_manifest(run_dir: &Path) -> Result<ExperimentManifest, String> {
    let path = run_dir.join(EXPERIMENT_FILE);
    let text =
        fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

// ── Per-cell path helpers ─────────────────────────────────────────────────────

/// Returns the path for a cell's schedule JSON inside `run_dir`.
pub fn schedule_path(run_dir: &Path, cell_id: &str) -> PathBuf {
    run_dir.join(SCHEDULES_DIR).join(format!("{cell_id}.json"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_handles_spaces_and_punctuation() {
        assert_eq!(experiment_slug("hello world"), "hello-world");
        assert_eq!(experiment_slug("ctao/paper:matrix"), "ctao-paper-matrix");
        assert_eq!(experiment_slug("-edges-"), "edges");
        assert_eq!(experiment_slug("***"), "experiment");
    }
}
