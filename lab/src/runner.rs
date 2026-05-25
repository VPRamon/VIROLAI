//! Parallel matrix experiment runner.
//!
//! [`execute`] is the main entry point for running an experiment matrix.  It:
//!
//! 1. Prepares each unique dataset once (loading JSON + running the prescheduler).
//! 2. Skips cells already marked `completed` in `state.jsonl` (resume mode).
//! 3. Dispatches pending cells to a bounded Rayon thread pool.
//! 4. When `no_state_log` is false, appends `started` / `completed` / `failed`
//!    events to `state.jsonl` as each cell executes.
//!    When `no_state_log` is true, prints per-cell progress to stderr instead.
//! 5. Returns a [`RunSummary`] with counts of total / skipped / completed /
//!    failed cells.

use chrono::Utc;
use rayon::prelude::*;
use schedulers::metrics::{MetricsContext, RankingWeights, ScheduleMetrics};
use schedulers::schedule::{LocationMeta, PeriodMeta, ScheduleMetadata, ScheduleOutput};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::cell::MatrixCell;
use crate::config::{HapSurvivorMode, RunConfig};
use crate::output;
use crate::problem::{PreparedProblem, prepare_problem};
use crate::spec::ExperimentSpec;
use crate::state::{CellStatus, StateEvent, StateWriter, completed_cells, read_events};

// ── Public API ────────────────────────────────────────────────────────────────

/// Summary returned by [`execute`].
pub struct RunSummary {
    /// Total number of cells in the matrix (including already-completed ones).
    pub total: usize,
    /// Cells skipped because they were already completed (resume mode).
    pub already_done: usize,
    /// Cells that completed successfully in this run.
    pub completed: usize,
    /// Cells that terminated with an error in this run.
    pub failed: usize,
    /// Path to the run directory.
    pub run_dir: PathBuf,
}

/// Executes all pending cells in `cells` with optional resume support.
///
/// When `resume` is `true`, cells already marked `completed` in `state.jsonl`
/// are skipped.  All other cells are dispatched to a Rayon thread pool sized
/// by `spec.max_parallel` (defaulting to the number of logical CPU cores).
///
/// When `no_state_log` is `true`, no `state.jsonl` is written; per-cell
/// progress is printed to `stderr` instead.  `resume` must be `false` when
/// `no_state_log` is `true`.
pub fn execute(
    spec: &ExperimentSpec,
    cells: &[MatrixCell],
    run_dir: &Path,
    resume: bool,
    no_state_log: bool,
) -> Result<RunSummary, String> {
    output::init_subdirs(run_dir)?;

    let already_done: HashSet<String> = if resume {
        let events = read_events(&run_dir.join(output::STATE_FILE))?;
        completed_cells(&events)
    } else {
        HashSet::new()
    };
    let pending: Vec<&MatrixCell> = cells
        .iter()
        .filter(|c| !already_done.contains(&c.cell_id))
        .collect();

    eprintln!(
        "lab: {} total cells, {} already completed, {} to run",
        cells.len(),
        already_done.len(),
        pending.len()
    );

    // Prepare each unique dataset exactly once.
    let mut prepared: HashMap<String, Arc<PreparedProblem>> = HashMap::new();
    for cell in &pending {
        if prepared.contains_key(&cell.dataset_id) {
            continue;
        }
        let p = prepare_problem(&cell.dataset_path, cell.horizon_override).map_err(|e| {
            format!(
                "dataset '{}' (path {}): {e}",
                cell.dataset_id,
                cell.dataset_path.display()
            )
        })?;
        prepared.insert(cell.dataset_id.clone(), Arc::new(p));
    }

    let max_parallel = spec
        .max_parallel
        .map(|n| n.max(1))
        .unwrap_or_else(|| std::cmp::max(1, std::cmp::min(num_cpus(), pending.len().max(1))));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(max_parallel)
        .build()
        .map_err(|e| format!("failed to build rayon pool: {e}"))?;

    let state_writer: Option<Arc<StateWriter>> = if no_state_log {
        None
    } else {
        Some(Arc::new(StateWriter::open_append(
            &run_dir.join(output::STATE_FILE),
        )?))
    };
    let ranking: Option<RankingWeights> = spec.ranking.map(Into::into);
    let run_dir_owned = run_dir.to_path_buf();

    let outcomes: Vec<CellOutcome> = pool.install(|| {
        pending
            .par_iter()
            .map(|cell| {
                let prepared = prepared
                    .get(&cell.dataset_id)
                    .expect("prepared dataset must exist")
                    .clone();
                run_one_cell(
                    cell,
                    &prepared,
                    &run_dir_owned,
                    ranking,
                    state_writer.as_deref(),
                )
            })
            .collect()
    });

    let mut completed = 0usize;
    let mut failed = 0usize;
    for o in &outcomes {
        match o {
            CellOutcome::Done { .. } => completed += 1,
            CellOutcome::Failed { .. } => failed += 1,
        }
    }

    Ok(RunSummary {
        total: cells.len(),
        already_done: already_done.len(),
        completed,
        failed,
        run_dir: run_dir.to_path_buf(),
    })
}

// ── Internal types ────────────────────────────────────────────────────────────

#[allow(dead_code)]
enum CellOutcome {
    Done { cell_id: String },
    Failed { cell_id: String, error: String },
}

// ── Cell execution ────────────────────────────────────────────────────────────

fn run_one_cell(
    cell: &MatrixCell,
    prepared: &PreparedProblem,
    run_dir: &Path,
    ranking: Option<RankingWeights>,
    state_writer: Option<&StateWriter>,
) -> CellOutcome {
    let started_at = Utc::now().to_rfc3339();
    if let Some(w) = state_writer {
        let _ = w.append(&StateEvent {
            cell_id: cell.cell_id.clone(),
            status: CellStatus::Started,
            schedule_path: None,
            error: None,
            started_at: started_at.clone(),
            finished_at: None,
        });
    } else {
        eprintln!("▶ {}", cell.cell_id);
    }

    match run_cell_inner(cell, prepared, run_dir, ranking) {
        Ok(paths) => {
            if let Some(w) = state_writer {
                let _ = w.append(&StateEvent {
                    cell_id: cell.cell_id.clone(),
                    status: CellStatus::Completed,
                    schedule_path: Some(paths.schedule_path.display().to_string()),
                    error: None,
                    started_at,
                    finished_at: Some(Utc::now().to_rfc3339()),
                });
            } else {
                eprintln!("✓ {}", cell.cell_id);
            }
            CellOutcome::Done {
                cell_id: cell.cell_id.clone(),
            }
        }
        Err(error) => {
            if let Some(w) = state_writer {
                let _ = w.append(&StateEvent {
                    cell_id: cell.cell_id.clone(),
                    status: CellStatus::Failed,
                    schedule_path: None,
                    error: Some(error.clone()),
                    started_at,
                    finished_at: Some(Utc::now().to_rfc3339()),
                });
            } else {
                eprintln!("✗ {}: {error}", cell.cell_id);
            }
            CellOutcome::Failed {
                cell_id: cell.cell_id.clone(),
                error,
            }
        }
    }
}

struct CellPaths {
    schedule_path: PathBuf,
}

fn run_cell_inner(
    cell: &MatrixCell,
    prepared: &PreparedProblem,
    run_dir: &Path,
    ranking: Option<RankingWeights>,
) -> Result<CellPaths, String> {
    let schedule_path = output::schedule_path(run_dir, &cell.cell_id);

    let scheduler_started = Instant::now();
    let (schedule,) = match cell.run_config {
        RunConfig::Est(config) => {
            let scheduler = config.build_scheduler()?;
            let schedule = scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("EST run {} failed: {e}", cell.cell_id))?;
            (schedule,)
        }
        RunConfig::Hap(config) => {
            let scheduler = config.build_scheduler()?;
            let schedule = scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("HAP run {} failed: {e}", cell.cell_id))?;
            (schedule,)
        }
    };
    let scheduler_runtime_ms = scheduler_started.elapsed().as_secs_f64() * 1000.0;

    let metadata = build_schedule_metadata(&cell.run_config, cell, prepared);
    let mut ctx = MetricsContext::new();
    if let Some(r) = ranking {
        ctx = ctx.with_ranking(r);
    }
    let mut metrics =
        ScheduleMetrics::compute(&schedule, &prepared.problem, &prepared.horizon, &ctx);
    metrics.scheduler_runtime_ms = Some(scheduler_runtime_ms);
    let metrics_value =
        serde_json::to_value(&metrics).map_err(|e| format!("failed to serialize metrics: {e}"))?;
    let output_obj = ScheduleOutput::new(prepared.raw_json.clone(), &schedule, Some(metadata))
        .with_metrics(metrics_value);
    let text = serde_json::to_string_pretty(&output_obj)
        .map_err(|e| format!("failed to serialize schedule {}: {e}", cell.cell_id))?;
    fs::write(&schedule_path, text)
        .map_err(|e| format!("failed to write {}: {e}", schedule_path.display()))?;

    Ok(CellPaths { schedule_path })
}

fn build_schedule_metadata(
    run: &RunConfig,
    cell: &MatrixCell,
    prepared: &PreparedProblem,
) -> ScheduleMetadata {
    let location = prepared.problem.telescope.as_ref().map(|t| LocationMeta {
        name: t.name.clone(),
        longitude_deg: t.location.lon.value(),
        latitude_deg: t.location.lat.value(),
        height_m: t.location.height.value(),
    });
    let period = Some(PeriodMeta {
        start_mjd_utc: prepared.horizon.start.value(),
        end_mjd_utc: prepared.horizon.end.value(),
    });
    let algorithm_config = match *run {
        RunConfig::Est(c) => serde_json::json!({
            "k_beams": c.k_beams,
            "branching_factor": c.branching_factor,
            "endangered_threshold": c.endangered_threshold,
            "fom": c.fom.to_string(),
        }),
        RunConfig::Hap(c) => {
            let survivor = match c.survivor_mode {
                HapSurvivorMode::GreedyOne => serde_json::json!({
                    "mode": c.survivor_mode.to_string()
                }),
                HapSurvivorMode::ElitistTopK | HapSurvivorMode::ParetoFront => {
                    serde_json::json!({
                        "mode": c.survivor_mode.to_string(),
                        "cap": c.survivor_cap,
                    })
                }
            };
            serde_json::json!({
                "iota_max": c.iota_max,
                "rho": c.rho,
                "population_size": c.population_size,
                "survivor": survivor,
                "seed": c.seed,
            })
        }
    };
    ScheduleMetadata {
        algorithm: run.algorithm().to_string(),
        algorithm_config,
        location,
        period,
        dataset_id: Some(cell.dataset_id.clone()),
        dataset_label: cell.dataset_label.clone(),
    }
}

/// Returns the number of logical CPU cores available.
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
