//! `experiment_matrix migrate` — port a legacy `est_experiment` run
//! directory into the new layout.

use scheduler::metrics::{MetricsContext, ScheduleMetrics};
use scheduler::schedule::{Schedule, SchedulingProblem};
use scheduler::time::{MJD, Period, Time};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cell::MatrixCell;
use crate::est_experiment::config::{
    EstRunConfig, HapRunConfig, HapSurvivorMode, HorizonOverride, RunConfig,
};
use crate::output::{self, SummaryRow};
use crate::spec::{AlgorithmSweep, DatasetEntry, ExperimentSpec};

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
    #[serde(default)]
    est_trace_jsonl: Option<String>,
}

pub struct MigrateSummary {
    pub cells_migrated: usize,
    pub run_dir: PathBuf,
}

pub fn migrate(old_run_dir: &Path, output: Option<&Path>) -> Result<MigrateSummary, String> {
    let manifest_path = old_run_dir.join("manifest.json");
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("failed to read {}: {e}", manifest_path.display()))?;
    let legacy: LegacyManifest = serde_json::from_str(&manifest_text)
        .map_err(|e| format!("failed to parse {}: {e}", manifest_path.display()))?;

    let dataset_path = PathBuf::from(&legacy.input_json);
    let dataset_id = dataset_id_from_path(&dataset_path);

    // Reconstruct cells.
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

    // Output dir
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
        emit_trace: cells
            .iter()
            .any(|c| matches!(c.run_config, RunConfig::Est(_))),
        max_parallel: None,
        output_dir: out_root.clone(),
    };

    let new_run_dir = output::create_run_dir(&out_root, &exp_name)?;
    output::init_subdirs(&new_run_dir)?;
    output::write_manifest(&new_run_dir, &spec, &cells)?;

    // Try to load each schedule and recompute full metrics; fall back to
    // zeroed metrics when the schedule JSON is unavailable.
    let problem_text = fs::read_to_string(&dataset_path).ok();
    let problem: Option<SchedulingProblem> = problem_text
        .as_ref()
        .and_then(|t| serde_json::from_str(t).ok());
    let horizon = legacy.horizon_override.and_then(|h| {
        if h.start_mjd.is_finite() && h.end_mjd.is_finite() && h.start_mjd < h.end_mjd {
            Some(Period::new(
                Time::<MJD>::new(h.start_mjd),
                Time::<MJD>::new(h.end_mjd),
            ))
        } else {
            None
        }
    });

    let mut summary_rows: Vec<SummaryRow> = Vec::new();
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
        if let Some(trace_rel) = run.est_trace_jsonl.as_ref() {
            let src = old_run_dir.join(trace_rel);
            if src.exists() {
                let dst = output::trace_path(&new_run_dir, &cell.cell_id);
                let _ = fs::copy(&src, &dst);
            }
        }

        let metrics = recompute_metrics(&new_schedule_path, problem.as_ref(), horizon.as_ref())
            .unwrap_or_else(zero_metrics);
        let m_text = serde_json::to_string_pretty(&metrics)
            .map_err(|e| format!("failed to serialize metrics for {}: {e}", cell.cell_id))?;
        fs::write(output::metrics_path(&new_run_dir, &cell.cell_id), m_text)
            .map_err(|e| format!("failed to write metrics: {e}"))?;
        summary_rows.push(SummaryRow::from_metrics(cell, &metrics));
    }

    output::write_summary_csv(&new_run_dir.join(output::SUMMARY_FILE), &summary_rows)?;

    Ok(MigrateSummary {
        cells_migrated: cells.len(),
        run_dir: new_run_dir,
    })
}

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

fn collect_est_axes(cells: &[MatrixCell]) -> Option<crate::est_experiment::config::EstSweepAxes> {
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
    Some(crate::est_experiment::config::EstSweepAxes {
        endangered_thresholds: e.into_iter().collect(),
        k_beams: k.into_iter().collect(),
        branching_factors: b.into_iter().collect(),
    })
}

fn collect_hap_axes(cells: &[MatrixCell]) -> Option<crate::est_experiment::config::HapSweepAxes> {
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
    Some(crate::est_experiment::config::HapSweepAxes {
        iota_max_values: iotas.into_iter().collect(),
        rho_values: rhos.into_iter().collect(),
        population_sizes: pops.into_iter().collect(),
        survivor_modes: modes.into_iter().collect(),
        survivor_caps: caps.into_iter().collect(),
        seeds: seeds.into_iter().collect(),
    })
}

fn recompute_metrics(
    schedule_path: &Path,
    problem: Option<&SchedulingProblem>,
    horizon: Option<&Period<MJD>>,
) -> Option<ScheduleMetrics> {
    let problem = problem?;
    let horizon = horizon.cloned().or(problem.detected_horizon)?;
    let text = fs::read_to_string(schedule_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;

    let mut schedule = Schedule::new();
    // 1. ScheduleOutput shape: tasks annotated inside scheduling_blocks.
    let blocks = value
        .get("scheduling_blocks")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .or_else(|| value.as_array().cloned())
        .unwrap_or_default();
    for block in &blocks {
        if let Some(tasks) = block.get("tasks").and_then(serde_json::Value::as_array) {
            for task in tasks {
                let scheduled = task
                    .get("scheduled")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if !scheduled {
                    continue;
                }
                let id = task.get("id").and_then(serde_json::Value::as_u64);
                let start = task
                    .get("scheduled_start_mjd_utc")
                    .and_then(serde_json::Value::as_f64);
                let end = task
                    .get("scheduled_end_mjd_utc")
                    .and_then(serde_json::Value::as_f64);
                if let (Some(id), Some(s), Some(e)) = (id, start, end) {
                    schedule.insert_placement(scheduler::schedule::TaskPlacement {
                        task_id: scheduler::time::TaskId(id),
                        start: Time::<MJD>::new(s),
                        end: Time::<MJD>::new(e),
                    });
                }
            }
        }
    }
    // 2. Fallback: explicit `schedule.placements` array — built manually
    // since `TaskPlacement` doesn't currently implement Deserialize.
    if schedule.is_empty()
        && let Some(arr) = value
            .get("schedule")
            .and_then(|s| s.get("placements"))
            .and_then(serde_json::Value::as_array)
    {
        for p in arr {
            let id = p.get("task_id").and_then(serde_json::Value::as_u64);
            let start = p.get("start").and_then(serde_json::Value::as_f64);
            let end = p.get("end").and_then(serde_json::Value::as_f64);
            if let (Some(id), Some(s), Some(e)) = (id, start, end) {
                schedule.insert_placement(scheduler::schedule::TaskPlacement {
                    task_id: scheduler::time::TaskId(id),
                    start: Time::<MJD>::new(s),
                    end: Time::<MJD>::new(e),
                });
            }
        }
    }

    Some(ScheduleMetrics::compute(
        &schedule,
        problem,
        &horizon,
        &MetricsContext::default(),
    ))
}

fn zero_metrics() -> ScheduleMetrics {
    use scheduler::metrics::{FragmentationStats, PriorityStats, RankingWeights, ResourceMetrics};
    ScheduleMetrics {
        scheduled_task_count: 0,
        total_task_count: 0,
        completion_ratio: 0.0,
        priority: PriorityStats::default(),
        fragmentation: FragmentationStats::default(),
        total_horizon_sec: 0.0,
        available_time_sec: 0.0,
        scheduled_time_sec: 0.0,
        utilization: 0.0,
        per_resource: Vec::<ResourceMetrics>::new(),
        composite_rank_score: 0.0,
        ranking_weights: RankingWeights::default(),
    }
}

#[allow(dead_code)]
fn _unused_anchor() {}

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
        // Tiny dummy schedule files so copy() succeeds.
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
        assert!(summary.run_dir.join("summary.csv").exists());
        let metrics_dir = summary.run_dir.join("metrics");
        let count = fs::read_dir(&metrics_dir).unwrap().count();
        assert_eq!(count, 2);
        let schedules_dir = summary.run_dir.join("schedules");
        let s_count = fs::read_dir(&schedules_dir).unwrap().count();
        assert_eq!(s_count, 2);
    }
}
