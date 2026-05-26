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
//!
//! Cache mode (enabled via [`RunOptions::cache`]) additionally:
//! - Computes a stable `run_key` for each pending cell from dataset content,
//!   algorithm, config, horizon, and version strings.
//! - Looks up the SQLite registry for already-completed keys.
//! - For registry hits, injects a synthetic `completed` state event (no
//!   schedule file) and increments `RunSummary::registry_hits`.
//! - For misses, runs the scheduler and inserts the result into the registry.

use chrono::Utc;
use rayon::prelude::*;
use schedulers::metrics::{MetricsContext, ScheduleMetrics};
use schedulers::schedule::{LocationMeta, PeriodMeta, ScheduleMetadata, ScheduleOutput};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::cell::MatrixCell;
use crate::config::{HapSurvivorMode, RunConfig};
use crate::output;
use crate::problem::{PreparedProblem, prepare_problem};
use crate::registry::{
    METRICS_VERSION, Registry, RunIdentity, hash_file, registry_path, scheduler_version,
};
use crate::spec::ExperimentSpec;
use crate::state::{CellStatus, StateEvent, StateWriter, completed_cells, read_events};

// ── Public API ────────────────────────────────────────────────────────────────

/// Summary returned by [`execute`] / [`execute_with_options`].
pub struct RunSummary {
    /// Total number of cells in the matrix (including already-completed ones).
    pub total: usize,
    /// Cells skipped because they were already completed (resume mode).
    pub already_done: usize,
    /// Cells served from the SQLite registry cache (no scheduler was run).
    pub registry_hits: usize,
    /// Cells that completed successfully in this run.
    pub completed: usize,
    /// Cells that terminated with an error in this run.
    pub failed: usize,
    /// Path to the run directory.
    pub run_dir: PathBuf,
}

/// Options for [`execute_with_options`].
#[derive(Debug, Default, Clone)]
pub struct RunOptions {
    /// Resume mode: skip cells already marked `completed` in `state.jsonl`.
    pub resume: bool,
    /// Suppress `state.jsonl`; print progress to stderr instead.
    pub no_state_log: bool,
    /// Enable SQLite registry cache.
    pub cache: bool,
    /// Path to the registry SQLite file.  Defaults to `.lab/runs.sqlite`.
    pub run_db: Option<PathBuf>,
    /// When true, do not create or write any filesystem artifacts (manifest,
    /// schedules, state). Results should still be stored in the registry DB.
    pub suppress_artifacts: bool,
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
///
/// This is a convenience wrapper around [`execute_with_options`] that keeps
/// the existing API stable.
pub fn execute(
    spec: &ExperimentSpec,
    cells: &[MatrixCell],
    run_dir: &Path,
    resume: bool,
    no_state_log: bool,
) -> Result<RunSummary, String> {
    execute_with_options(
        spec,
        cells,
        run_dir,
        RunOptions {
            resume,
            no_state_log,
            cache: false,
            run_db: None,
        },
    )
}

/// Executes all pending cells according to [`RunOptions`].
pub fn execute_with_options(
    spec: &ExperimentSpec,
    cells: &[MatrixCell],
    run_dir: &Path,
    opts: RunOptions,
) -> Result<RunSummary, String> {
    if opts.no_state_log && opts.resume {
        return Err("--no-state and --resume are mutually exclusive".to_string());
    }

    if !opts.suppress_artifacts {
        output::init_subdirs(run_dir)?;
    }

    let already_done: HashSet<String> = if opts.resume {
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

    // ── Registry cache lookup ─────────────────────────────────────────────────
    // Partition pending cells into registry hits and misses.
    let (registry_hit_ids, cells_to_run): (HashSet<String>, Vec<&MatrixCell>) = if opts.cache {
        let db_path = registry_path(opts.run_db.as_deref());
        let registry = Registry::open(&db_path)?;

        let sched_ver = scheduler_version();
        let mut hits = HashSet::new();
        let mut misses = Vec::new();

        for cell in &pending {
            match build_identity(cell, &sched_ver) {
                Ok(identity) => {
                    let key = identity.run_key();
                    if registry.contains(&key)? {
                        hits.insert(cell.cell_id.clone());
                    } else {
                        misses.push(*cell);
                    }
                }
                Err(_) => {
                    // If identity computation fails (e.g. file unreadable),
                    // fall through to normal execution.
                    misses.push(*cell);
                }
            }
        }

        if !hits.is_empty() {
            eprintln!("lab: {} registry cache hits", hits.len());
        }
        (hits, misses)
    } else {
        (HashSet::new(), pending.clone())
    };

    // Emit synthetic completed events for registry hits.
    let state_writer: Option<Arc<StateWriter>> = if opts.no_state_log || opts.suppress_artifacts {
        None
    } else {
        Some(Arc::new(StateWriter::open_append(
            &run_dir.join(output::STATE_FILE),
        )?))
    };

    for cell in cells
        .iter()
        .filter(|c| registry_hit_ids.contains(&c.cell_id))
    {
        let now = Utc::now().to_rfc3339();
        if let Some(w) = &state_writer {
            let _ = w.append(&StateEvent {
                cell_id: cell.cell_id.clone(),
                status: CellStatus::Completed,
                schedule_path: None, // no schedule written for cache hits
                error: None,
                started_at: now.clone(),
                finished_at: Some(now),
            });
        } else {
            eprintln!("● {} (registry cache hit)", cell.cell_id);
        }
    }

    let max_parallel = spec
        .max_parallel
        .map(|n| n.max(1))
        .unwrap_or_else(|| std::cmp::max(1, std::cmp::min(num_cpus(), cells_to_run.len().max(1))));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(max_parallel)
        .build()
        .map_err(|e| format!("failed to build rayon pool: {e}"))?;

    let run_dir_owned = run_dir.to_path_buf();

    // Shared registry for miss-path inserts.
    let registry_arc: Option<Arc<Mutex<Registry>>> = if opts.cache {
        let db_path = registry_path(opts.run_db.as_deref());
        Some(Arc::new(Mutex::new(Registry::open(&db_path)?)))
    } else {
        None
    };
    let sched_ver = scheduler_version();

    let suppress = opts.suppress_artifacts;
    let outcomes: Vec<CellOutcome> = pool.install(|| {
        cells_to_run
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
                    state_writer.as_deref(),
                    registry_arc.as_deref(),
                    &sched_ver,
                    suppress,
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
        registry_hits: registry_hit_ids.len(),
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
    state_writer: Option<&StateWriter>,
    registry: Option<&Mutex<Registry>>,
    sched_ver: &str,
    suppress_artifacts: bool,
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

    match run_cell_inner_impl(cell, prepared, run_dir, registry, sched_ver, suppress_artifacts) {
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
    registry: Option<&Mutex<Registry>>,
    sched_ver: &str,
) -> Result<CellPaths, String> {
    // shim kept for compatibility; this function now forwards to the
    // inner implementation below that takes the suppress flag.
    run_cell_inner_impl(cell, prepared, run_dir, registry, sched_ver, false)
}

fn run_cell_inner_impl(
    cell: &MatrixCell,
    prepared: &PreparedProblem,
    run_dir: &Path,
    registry: Option<&Mutex<Registry>>,
    sched_ver: &str,
    suppress_artifacts: bool,
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
        RunConfig::Lst(config) => {
            let scheduler = config.build_scheduler()?;
            let schedule = scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("LST run {} failed: {e}", cell.cell_id))?;
            (schedule,)
        }
    };
    let scheduler_runtime_ms = scheduler_started.elapsed().as_secs_f64() * 1000.0;

    let metadata = build_schedule_metadata(&cell.run_config, cell, prepared);
    let ctx = MetricsContext::new();
    let mut metrics =
        ScheduleMetrics::compute(&schedule, &prepared.problem, &prepared.horizon, &ctx);
    metrics.scheduler_runtime_ms = Some(scheduler_runtime_ms);
    let metrics_value =
        serde_json::to_value(&metrics).map_err(|e| format!("failed to serialize metrics: {e}"))?;
    let output_obj = ScheduleOutput::new(prepared.raw_json.clone(), &schedule, Some(metadata))
        .with_metrics(metrics_value);
    let text = serde_json::to_string_pretty(&output_obj)
        .map_err(|e| format!("failed to serialize schedule {}: {e}", cell.cell_id))?;
    if !suppress_artifacts {
        fs::write(&schedule_path, text)
            .map_err(|e| format!("failed to write {}: {e}", schedule_path.display()))?;
    }

    // Insert into registry on success (cache mode only).
    if let Some(reg_mutex) = registry && let Ok(identity) = build_identity(cell, sched_ver) {
        let metrics_json = serde_json::to_string(&metrics).unwrap_or_else(|_| "{}".to_string());
        if let Ok(reg) = reg_mutex.lock() {
            let _ = reg.upsert(&identity, &metrics_json, Some(&cell.cell_id));
        }
    }

    Ok(CellPaths { schedule_path })
}

/// Builds a [`RunIdentity`] for a cell.
pub(crate) fn build_identity(cell: &MatrixCell, sched_ver: &str) -> Result<RunIdentity, String> {
    let dataset_hash = hash_file(&cell.dataset_path)?;
    let config_json = serde_json::to_string(&cell.run_config)
        .map_err(|e| format!("failed to serialize run config: {e}"))?;
    let horizon_json = cell
        .horizon_override
        .map(|h| serde_json::to_string(&h))
        .transpose()
        .map_err(|e: serde_json::Error| format!("failed to serialize horizon: {e}"))?;
    Ok(RunIdentity {
        dataset_id: cell.dataset_id.clone(),
        dataset_path: cell.dataset_path.display().to_string(),
        dataset_hash,
        algorithm: cell.algorithm.clone(),
        config_slug: cell.run_config.slug(),
        config_json,
        horizon_json,
        scheduler_version: sched_ver.to_string(),
        metrics_version: METRICS_VERSION.to_string(),
    })
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
        RunConfig::Lst(c) => serde_json::json!({
            "k_beams": c.k_beams,
            "branching_factor": c.branching_factor,
            "endangered_threshold": c.endangered_threshold,
            "fom": c.fom.to_string(),
        }),
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
