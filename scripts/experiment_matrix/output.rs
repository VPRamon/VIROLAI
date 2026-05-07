//! Output directory layout, summary CSV, and resolved-spec manifest.

use chrono::Utc;
use scheduler::metrics::ScheduleMetrics;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::cell::MatrixCell;
use crate::spec::ExperimentSpec;

pub const SCHEDULES_DIR: &str = "schedules";
pub const METRICS_DIR: &str = "metrics";
pub const TRACES_DIR: &str = "traces";
pub const STATE_FILE: &str = "state.jsonl";
pub const SUMMARY_FILE: &str = "summary.csv";
pub const EXPERIMENT_FILE: &str = "experiment.json";

/// Slugify the experiment name into a filesystem-safe directory component.
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

/// Create `<output_dir>/<experiment_slug>/run-<ts>/` plus `schedules/`,
/// `metrics/`, and (always) `traces/` subdirectories.
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

/// Ensure the layout subdirectories exist inside an existing `run-*` dir.
pub fn init_subdirs(run_dir: &Path) -> Result<(), String> {
    for sub in [SCHEDULES_DIR, METRICS_DIR, TRACES_DIR] {
        let p = run_dir.join(sub);
        fs::create_dir_all(&p).map_err(|e| format!("failed to create {}: {e}", p.display()))?;
    }
    Ok(())
}

/// The serialized spec + cell list. Written to `experiment.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentManifest {
    pub spec: ExperimentSpec,
    pub cells: Vec<MatrixCell>,
}

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

/// Read an `experiment.json` produced by [`write_manifest`].
#[allow(dead_code)]
pub fn read_manifest(run_dir: &Path) -> Result<ExperimentManifest, String> {
    let path = run_dir.join(EXPERIMENT_FILE);
    let text =
        fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

/// Flat row of scalar metrics for `summary.csv`.
#[derive(Debug, Clone, Serialize)]
pub struct SummaryRow {
    pub cell_id: String,
    pub dataset_id: String,
    pub algorithm: String,
    pub config_slug: String,
    pub scheduled_task_count: usize,
    pub total_task_count: usize,
    pub completion_ratio: f64,
    pub priority_sum: f64,
    pub priority_min: f64,
    pub priority_max: f64,
    pub priority_mean: f64,
    pub priority_std: f64,
    pub priority_p25: f64,
    pub priority_p50: f64,
    pub priority_p75: f64,
    pub priority_p90: f64,
    pub fragmentation_gap_count: usize,
    pub fragmentation_gap_total_sec: f64,
    pub fragmentation_largest_gap_sec: f64,
    pub fragmentation_index: f64,
    pub total_horizon_sec: f64,
    pub available_time_sec: f64,
    pub scheduled_time_sec: f64,
    pub utilization: f64,
    pub composite_rank_score: f64,
}

impl SummaryRow {
    pub fn from_metrics(cell: &MatrixCell, m: &ScheduleMetrics) -> Self {
        Self {
            cell_id: cell.cell_id.clone(),
            dataset_id: cell.dataset_id.clone(),
            algorithm: cell.algorithm.clone(),
            config_slug: cell.run_config.slug(),
            scheduled_task_count: m.scheduled_task_count,
            total_task_count: m.total_task_count,
            completion_ratio: m.completion_ratio,
            priority_sum: m.priority.sum,
            priority_min: m.priority.min,
            priority_max: m.priority.max,
            priority_mean: m.priority.mean,
            priority_std: m.priority.std,
            priority_p25: m.priority.p25,
            priority_p50: m.priority.p50,
            priority_p75: m.priority.p75,
            priority_p90: m.priority.p90,
            fragmentation_gap_count: m.fragmentation.gap_count,
            fragmentation_gap_total_sec: m.fragmentation.gap_total_sec,
            fragmentation_largest_gap_sec: m.fragmentation.largest_gap_sec,
            fragmentation_index: m.fragmentation.fragmentation_index,
            total_horizon_sec: m.total_horizon_sec,
            available_time_sec: m.available_time_sec,
            scheduled_time_sec: m.scheduled_time_sec,
            utilization: m.utilization,
            composite_rank_score: m.composite_rank_score,
        }
    }
}

/// Write `summary.csv` from a list of rows. Always overwrites.
pub fn write_summary_csv(path: &Path, rows: &[SummaryRow]) -> Result<(), String> {
    let mut writer = csv::Writer::from_path(path)
        .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    for row in rows {
        writer
            .serialize(row)
            .map_err(|e| format!("failed to write summary row: {e}"))?;
    }
    writer
        .flush()
        .map_err(|e| format!("failed to flush {}: {e}", path.display()))
}

// Path helpers ---------------------------------------------------------------

pub fn schedule_path(run_dir: &Path, cell_id: &str) -> PathBuf {
    run_dir.join(SCHEDULES_DIR).join(format!("{cell_id}.json"))
}

pub fn metrics_path(run_dir: &Path, cell_id: &str) -> PathBuf {
    run_dir.join(METRICS_DIR).join(format!("{cell_id}.json"))
}

pub fn trace_path(run_dir: &Path, cell_id: &str) -> PathBuf {
    run_dir.join(TRACES_DIR).join(format!("{cell_id}.jsonl"))
}

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
