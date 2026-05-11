//! Legacy run-directory migration.
//!
//! `experiments migrate <old_run_dir>` ports a run directory produced by
//! the deprecated `est_experiment` binary into the current
//! `experiment_matrix`-compatible layout.
//!
//! # Layout differences
//!
//! The legacy format used a flat `schedules/` directory with a single
//! `manifest.json` listing run slugs.  The new format organises output under
//! `<experiment>/<run-timestamp>/` with per-cell `schedules/` and
//! `experiment.json` and `state.jsonl`.
//!
//! Migration reconstructs the [`MatrixCell`] list from the legacy manifest,
//! copies schedule files, and writes the full modern layout.

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cell::MatrixCell;
use crate::config::{
    EstRunConfig, EstSweepAxes, HapRunConfig, HapSurvivorMode, HapSweepAxes, HorizonOverride,
    RunConfig,
};
use crate::output;
use crate::spec::{AlgorithmSweep, DatasetEntry, ExperimentSpec};

// ── Legacy manifest types (deserialization only) ──────────────────────────────

#[derive(Debug, Deserialize)]
struct LegacyManifest {
    input_json: String,
    #[serde(default)]
    horizon_override: Option<HorizonOverride>,
    #[serde(default)]
    runs: Vec<LegacyRunEntry>,
}

#[derive(Debug, Deserialize)]
struct LegacyRunEntry {
    slug: String,
    algorithm: String,
    #[serde(default)]
    endangered_threshold: Option<u32>,
    #[serde(default)]
    k_beams: Option<usize>,
    #[serde(default)]
    branching_factor: Option<usize>,
    #[serde(default)]
    iota_max: Option<usize>,
    #[serde(default)]
    rho: Option<usize>,
    #[serde(default)]
    population_size: Option<usize>,
    #[serde(default)]
    survivor_mode: Option<HapSurvivorMode>,
    #[serde(default)]
    survivor_cap: Option<usize>,
    #[serde(default)]
    seed: Option<u64>,
    schedule_json: String,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Summary returned by [`migrate`].
pub struct MigrateSummary {
    /// Number of cells successfully migrated.
    pub cells_migrated: usize,
    /// Path to the newly created run directory.
    pub run_dir: PathBuf,
}

/// Migrates a legacy `est_experiment` run directory at `old_run_dir` into a
/// new run directory under `output` (or `<old_run_dir>/migrated` when `None`).
///
/// # Errors
///
/// Returns an error if the legacy `manifest.json` is missing, malformed, or
/// references an unknown algorithm.
pub fn migrate(old_run_dir: &Path, output: Option<&Path>) -> Result<MigrateSummary, String> {
    let manifest_path = old_run_dir.join("manifest.json");
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("failed to read {}: {e}", manifest_path.display()))?;
    let legacy: LegacyManifest = serde_json::from_str(&manifest_text)
        .map_err(|e| format!("failed to parse {}: {e}", manifest_path.display()))?;

    let dataset_path = PathBuf::from(&legacy.input_json);
    let dataset_id = dataset_id_from_path(&dataset_path);

    // Reconstruct cells from legacy run entries.
    let mut cells = Vec::with_capacity(legacy.runs.len());
    for run in &legacy.runs {
        let run_config = legacy_run_to_config(run)?;
        let cell_id = format!("{}__{}__{}", dataset_id, run.algorithm, run_config.slug());
        cells.push(MatrixCell {
            cell_id,
            dataset_id: dataset_id.clone(),
            dataset_path: dataset_path.clone(),
            dataset_label: None,
            horizon_override: legacy.horizon_override,
            algorithm: run.algorithm.clone(),
            run_config,
        });
    }

    let est_axes = collect_est_axes(&cells);
    let hap_axes = collect_hap_axes(&cells);
    let mut algorithms: Vec<AlgorithmSweep> = Vec::new();
    if let Some(a) = est_axes {
        algorithms.push(AlgorithmSweep::Est { axes: a });
    }
    if let Some(a) = hap_axes {
        algorithms.push(AlgorithmSweep::Hap { axes: a });
    }

    let out_root = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| old_run_dir.join("migrated"));
    let exp_name = format!(
        "migrated-{}",
        old_run_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("legacy")
    );
    let spec = ExperimentSpec {
        name: exp_name.clone(),
        datasets: vec![DatasetEntry {
            id: dataset_id.clone(),
            path: dataset_path.clone(),
            label: None,
            horizon_override: legacy.horizon_override,
        }],
        algorithms,
        ranking: None,
        max_parallel: None,
        output_dir: out_root.clone(),
    };

    let new_run_dir = output::create_run_dir(&out_root, &exp_name)?;
    output::init_subdirs(&new_run_dir)?;
    output::write_manifest(&new_run_dir, &spec, &cells)?;

    let mut cells_migrated = 0usize;
    for (cell, run) in cells.iter().zip(legacy.runs.iter()) {
        let legacy_schedule_path = old_run_dir.join(&run.schedule_json);
        let new_schedule_path = output::schedule_path(&new_run_dir, &cell.cell_id);
        if legacy_schedule_path.exists() {
            fs::copy(&legacy_schedule_path, &new_schedule_path).map_err(|e| {
                format!(
                    "failed to copy schedule {} -> {}: {e}",
                    legacy_schedule_path.display(),
                    new_schedule_path.display()
                )
            })?;
        }

        cells_migrated += 1;
    }

    Ok(MigrateSummary {
        cells_migrated,
        run_dir: new_run_dir,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Derives a filesystem-safe dataset slug from a file path stem.
fn dataset_id_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dataset");
    let mut out = String::with_capacity(stem.len());
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "dataset".to_string()
    } else {
        out
    }
}

/// Reconstructs a [`RunConfig`] from a legacy manifest run entry.
fn legacy_run_to_config(run: &LegacyRunEntry) -> Result<RunConfig, String> {
    match run.algorithm.as_str() {
        "est" => {
            let def = EstRunConfig::default();
            Ok(RunConfig::Est(EstRunConfig {
                fom: def.fom,
                endangered_threshold: run.endangered_threshold.unwrap_or(def.endangered_threshold),
                k_beams: run.k_beams.unwrap_or(def.k_beams),
                branching_factor: run.branching_factor.unwrap_or(def.branching_factor),
            }))
        }
        "hap" => {
            let def = HapRunConfig::default();
            Ok(RunConfig::Hap(HapRunConfig {
                iota_max: run.iota_max.unwrap_or(def.iota_max),
                rho: run.rho.unwrap_or(def.rho),
                population_size: run.population_size.unwrap_or(def.population_size),
                survivor_mode: run.survivor_mode.unwrap_or(def.survivor_mode),
                survivor_cap: run.survivor_cap.unwrap_or(def.survivor_cap),
                seed: run.seed.unwrap_or(def.seed),
            }))
        }
        other => Err(format!(
            "unknown legacy algorithm '{other}' in slug '{}'",
            run.slug
        )),
    }
}

fn collect_est_axes(cells: &[MatrixCell]) -> Option<EstSweepAxes> {
    let mut e = std::collections::BTreeSet::new();
    let mut k = std::collections::BTreeSet::new();
    let mut b = std::collections::BTreeSet::new();
    let mut any = false;
    for c in cells {
        if let RunConfig::Est(cfg) = c.run_config {
            any = true;
            e.insert(cfg.endangered_threshold);
            k.insert(cfg.k_beams);
            b.insert(cfg.branching_factor);
        }
    }
    if !any {
        return None;
    }
    Some(EstSweepAxes {
        endangered_thresholds: e.into_iter().collect(),
        k_beams: k.into_iter().collect(),
        branching_factors: b.into_iter().collect(),
        foms: vec![],
    })
}

fn collect_hap_axes(cells: &[MatrixCell]) -> Option<HapSweepAxes> {
    let mut iotas = std::collections::BTreeSet::new();
    let mut rhos = std::collections::BTreeSet::new();
    let mut pops = std::collections::BTreeSet::new();
    let mut modes = std::collections::BTreeSet::new();
    let mut caps = std::collections::BTreeSet::new();
    let mut seeds = std::collections::BTreeSet::new();
    let mut any = false;
    for c in cells {
        if let RunConfig::Hap(cfg) = c.run_config {
            any = true;
            iotas.insert(cfg.iota_max);
            rhos.insert(cfg.rho);
            pops.insert(cfg.population_size);
            modes.insert(cfg.survivor_mode);
            caps.insert(cfg.survivor_cap);
            seeds.insert(cfg.seed);
        }
    }
    if !any {
        return None;
    }
    Some(HapSweepAxes {
        iota_max_values: iotas.into_iter().collect(),
        rho_values: rhos.into_iter().collect(),
        population_sizes: pops.into_iter().collect(),
        survivor_modes: modes.into_iter().collect(),
        survivor_caps: caps.into_iter().collect(),
        seeds: seeds.into_iter().collect(),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_legacy_run_dir(root: &Path) -> PathBuf {
        let run_dir = root.join("run-legacy");
        fs::create_dir_all(run_dir.join("schedules")).unwrap();
        let manifest = serde_json::json!({
            "input_json": "data/missing.json",
            "output_dir": "out/",
            "horizon_override": null,
            "baseline_slug": "e1-k1-b1",
            "comparison_csv": "comparison.csv",
            "runs": [
                {
                    "slug": "e1-k1-b1",
                    "algorithm": "est",
                    "endangered_threshold": 1,
                    "k_beams": 1,
                    "branching_factor": 1,
                    "schedule_json": "schedules/e1-k1-b1.json"
                },
                {
                    "slug": "hap-i64-r2-p4-elitist4-s0",
                    "algorithm": "hap",
                    "iota_max": 64,
                    "rho": 2,
                    "population_size": 4,
                    "survivor_mode": "elitist_top_k",
                    "survivor_cap": 4,
                    "seed": 0,
                    "schedule_json": "schedules/hap.json"
                }
            ]
        });
        fs::write(
            run_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let dummy = serde_json::json!({"schedule": {"placements": []}, "metadata": null});
        fs::write(
            run_dir.join("schedules/e1-k1-b1.json"),
            serde_json::to_string(&dummy).unwrap(),
        )
        .unwrap();
        fs::write(
            run_dir.join("schedules/hap.json"),
            serde_json::to_string(&dummy).unwrap(),
        )
        .unwrap();
        run_dir
    }

    #[test]
    fn migrate_happy_path_creates_layout() {
        let tmp = TempDir::new().unwrap();
        let legacy = write_legacy_run_dir(tmp.path());
        let out = tmp.path().join("new");
        let summary = migrate(&legacy, Some(&out)).expect("migrate ok");
        assert_eq!(summary.cells_migrated, 2);
        assert!(summary.run_dir.join("experiment.json").exists());
        // metrics/ is no longer written; metrics are embedded in schedule JSONs.
        assert!(!summary.run_dir.join("metrics").exists());
        let schedules_dir = summary.run_dir.join("schedules");
        let s_count = fs::read_dir(&schedules_dir).unwrap().count();
        assert_eq!(s_count, 2);
    }
}
