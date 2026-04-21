use chrono::Utc;
use csv::Writer;
use serde::Serialize;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::config::{EstRunConfig, HorizonOverride};
use super::resolve::ResolvedEstExperiment;
use super::run::RunOutcome;

/// Returned by [`run_experiment`] after all runs complete.
///
/// [`run_experiment`]: super::run_experiment
#[derive(Debug, Clone)]
pub struct EstExperimentExecution {
    pub output_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub comparison_csv_path: PathBuf,
    pub schedule_paths: Vec<PathBuf>,
    pub run_count: usize,
}

/// Top-level manifest written to `manifest.json` in the output directory.
#[derive(Debug, Clone, Serialize)]
pub struct EstExperimentManifest {
    pub input_json: String,
    pub output_dir: String,
    pub horizon_override: Option<HorizonOverride>,
    pub baseline_slug: String,
    pub baseline: EstRunConfig,
    /// Path to the comparison CSV, relative to the output directory.
    pub comparison_csv: String,
    pub runs: Vec<ManifestRunEntry>,
}

/// One entry per run in the manifest.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestRunEntry {
    pub slug: String,
    pub fom: crate::scheduler::est::EstFomKind,
    pub endangered_threshold: u32,
    pub k_beams: usize,
    pub branching_factor: usize,
    /// Path to the schedule JSON, relative to the output directory.
    pub schedule_json: String,
}

/// A compact row for the EST comparison CSV.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ComparisonRow {
    run_slug: String,
    is_baseline: bool,
    scheduled_task_count: usize,
    fitness_priority_sum: f64,
    scheduled_priority_p25: f64,
    scheduled_priority_p50: f64,
    scheduled_priority_p75: f64,
    scheduled_priority_p90: f64,
}

pub(crate) fn build_manifest(
    experiment: &ResolvedEstExperiment,
    output_dir: &Path,
    comparison_csv_path: &Path,
    outcomes: &[RunOutcome],
) -> EstExperimentManifest {
    EstExperimentManifest {
        input_json: experiment.input_path.display().to_string(),
        output_dir: output_dir.display().to_string(),
        horizon_override: experiment.horizon_override,
        baseline_slug: experiment.baseline_slug(),
        baseline: experiment.baseline,
        comparison_csv: relative_to_output(output_dir, comparison_csv_path),
        runs: outcomes
            .iter()
            .map(|outcome| ManifestRunEntry {
                slug: outcome.config.slug(),
                fom: outcome.config.fom,
                endangered_threshold: outcome.config.endangered_threshold,
                k_beams: outcome.config.k_beams,
                branching_factor: outcome.config.branching_factor,
                schedule_json: relative_to_output(output_dir, &outcome.schedule_path),
            })
            .collect(),
    }
}

pub(crate) fn build_comparison_row(baseline_slug: &str, outcome: &RunOutcome) -> ComparisonRow {
    let is_baseline = outcome.config.slug() == baseline_slug;

    ComparisonRow {
        run_slug: outcome.config.schedule_file_stem(),
        is_baseline,
        scheduled_task_count: outcome.metrics.scheduled_task_count,
        fitness_priority_sum: outcome.metrics.fitness_priority_sum,
        scheduled_priority_p25: outcome.metrics.scheduled_priority_p25,
        scheduled_priority_p50: outcome.metrics.scheduled_priority_p50,
        scheduled_priority_p75: outcome.metrics.scheduled_priority_p75,
        scheduled_priority_p90: outcome.metrics.scheduled_priority_p90,
    }
}

pub(crate) fn write_comparison_csv(path: &Path, rows: &[ComparisonRow]) -> Result<(), String> {
    let mut writer = Writer::from_path(path)
        .map_err(|e| format!("failed to create comparison CSV {}: {e}", path.display()))?;
    for row in rows {
        writer
            .serialize(row)
            .map_err(|e| format!("failed to write comparison row to {}: {e}", path.display()))?;
    }
    writer
        .flush()
        .map_err(|e| format!("failed to flush comparison CSV {}: {e}", path.display()))
}

/// Validates or creates the base output directory and returns a fresh timestamped run directory.
pub(crate) fn prepare_output_dir(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!(
                "output path {} exists and is not a directory",
                path.display()
            ));
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|e| format!("failed to create output directory {}: {e}", path.display()))?;
    }

    let now = Utc::now();
    let base_name = format!(
        "run-{}-{:09}Z",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    );

    for suffix in 0..100 {
        let run_name = if suffix == 0 {
            base_name.clone()
        } else {
            format!("{base_name}-{suffix}")
        };
        let run_dir = path.join(run_name);
        match fs::create_dir(&run_dir) {
            Ok(()) => return Ok(run_dir),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create timestamped output directory {}: {error}",
                    run_dir.display()
                ));
            }
        }
    }

    Err(format!(
        "failed to create unique timestamped output directory under {}",
        path.display()
    ))
}

fn relative_to_output(output_dir: &Path, path: &Path) -> String {
    path.strip_prefix(output_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn prepare_output_dir_creates_timestamped_child_directory() {
        let temp_dir = TempDir::new().expect("temp dir should exist");

        let run_dir = prepare_output_dir(temp_dir.path()).expect("run directory should be created");
        assert!(run_dir.is_dir());
        assert_eq!(run_dir.parent(), Some(temp_dir.path()));

        let run_dir_name = run_dir
            .file_name()
            .and_then(|name| name.to_str())
            .expect("run directory should be valid UTF-8");
        assert!(run_dir_name.starts_with("run-"));
    }

    #[test]
    fn prepare_output_dir_accepts_non_empty_base_directory() {
        let temp_dir = TempDir::new().expect("temp dir should exist");
        fs::write(temp_dir.path().join("existing.txt"), "from previous run")
            .expect("fixture file should be written");

        let run_dir = prepare_output_dir(temp_dir.path()).expect("run directory should be created");
        assert!(run_dir.is_dir());
    }

    #[test]
    fn prepare_output_dir_rejects_non_directory_path() {
        let temp_dir = TempDir::new().expect("temp dir should exist");
        let output_file = temp_dir.path().join("output-file");
        fs::write(&output_file, "not a directory").expect("output file should be written");

        let error =
            prepare_output_dir(Path::new(&output_file)).expect_err("path should be rejected");
        assert!(error.contains("is not a directory"));
    }
}
