//! Parallel matrix experiment runner — DB-only mode.
//!
//! [`execute`] is the main entry point for running an experiment matrix.  It:
//!
//! 1. Prepares each unique dataset once (loading JSON + running the
//!    prescheduler).
//! 2. Opens (or creates) the SQLite registry and checks each cell's
//!    [`RunIdentity`] hash against existing rows.
//! 3. Skips DB hits unless [`RunOptions::override_existing`] is set.
//! 4. Dispatches pending cells to a bounded Rayon thread pool.
//! 5. After each successful run, upserts metrics **and** the full schedule
//!    JSON into the registry.
//! 6. Refreshes a single-line progress indicator on stderr (TTY) or prints
//!    compact summaries (non-TTY).
//!
//! No filesystem artifacts (schedules directory, state.jsonl, manifests) are
//! written.  All persistent state lives in SQLite.

use rayon::prelude::*;
use schedulers::metrics::{MetricsContext, ScheduleMetrics};
use schedulers::schedule::{LocationMeta, PeriodMeta, ScheduleMetadata, ScheduleOutput};
use std::collections::HashMap;
use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::experiment::cell::MatrixCell;
use crate::experiment::config::{HapSurvivorMode, RunConfig};
use crate::experiment::problem::{PreparedProblem, prepare_problem};
use crate::experiment::spec::ExperimentSpec;
use crate::registry::{
    METRICS_VERSION, Registry, RunIdentity, canonical_schedule_hash, hash_file, registry_path,
    scheduler_version,
};

// ── Public API ────────────────────────────────────────────────────────────────

/// Summary returned by [`execute`].
pub struct RunSummary {
    /// Total number of cells in the matrix.
    pub total: usize,
    /// Cells skipped because a matching row already existed in the registry
    /// and `override_existing` was false.
    pub skipped: usize,
    /// Cells that were re-executed because `override_existing` was true and a
    /// matching row already existed.
    pub overridden: usize,
    /// Cells that completed successfully and were upserted into the registry.
    pub completed: usize,
    /// Cells that terminated with an error.
    pub failed: usize,
}

/// Options for [`execute`].
#[derive(Debug, Default, Clone)]
pub struct RunOptions {
    /// Path to the registry SQLite file.  Defaults to `.lab/runs.sqlite`.
    pub run_db: Option<PathBuf>,
    /// When true, re-execute cells that already have a row in the registry and
    /// overwrite their stored metrics and schedule JSON.
    pub override_existing: bool,
}

/// Executes the experiment matrix described by `cells`, storing results in the
/// SQLite registry identified by `opts.run_db`.
pub fn execute(
    spec: &ExperimentSpec,
    cells: &[MatrixCell],
    opts: RunOptions,
) -> Result<RunSummary, String> {
    let db_path = registry_path(opts.run_db.as_deref());
    let registry = Registry::open(&db_path)?;
    let sched_ver = scheduler_version();

    // ── Classify each cell as skip, override, or fresh run ───────────────────
    let mut skip_count = 0usize;
    let mut override_count_pre = 0usize;
    let cells_to_run: Vec<&MatrixCell> = cells
        .iter()
        .filter(|c| match build_identity(c, &sched_ver) {
            Ok(identity) => {
                let key = identity.run_key();
                match registry.contains(&key) {
                    Ok(true) => {
                        if opts.override_existing {
                            override_count_pre += 1;
                            true
                        } else {
                            skip_count += 1;
                            false
                        }
                    }
                    _ => true, // registry miss (or error) → run it
                }
            }
            Err(_) => true, // identity failure → attempt execution
        })
        .collect();

    let total = cells.len();
    eprintln!(
        "lab: {} total, {} skipped, {} to run",
        total,
        skip_count,
        cells_to_run.len()
    );

    // ── Prepare each unique dataset exactly once ──────────────────────────────
    let mut prepared: HashMap<String, Arc<PreparedProblem>> = HashMap::new();
    for cell in &cells_to_run {
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

    // ── Rayon pool ────────────────────────────────────────────────────────────
    let max_parallel = spec
        .max_parallel
        .map(|n| n.max(1))
        .unwrap_or_else(|| std::cmp::max(1, std::cmp::min(num_cpus(), cells_to_run.len().max(1))));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(max_parallel)
        .build()
        .map_err(|e| format!("failed to build rayon pool: {e}"))?;

    let registry_arc = Arc::new(Mutex::new(registry));
    let done = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicUsize::new(0));
    let is_tty = std::io::stderr().is_terminal();
    let run_total = cells_to_run.len();

    let outcomes: Vec<CellOutcome> = pool.install(|| {
        cells_to_run
            .par_iter()
            .map(|cell| {
                let prepared = prepared
                    .get(&cell.dataset_id)
                    .expect("prepared dataset must exist")
                    .clone();
                let outcome = run_one_cell(cell, &prepared, &registry_arc, &sched_ver);
                // Update progress counters and refresh the stderr line.
                let (d, f) = match &outcome {
                    CellOutcome::Done => {
                        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                        let f = fail.load(Ordering::Relaxed);
                        (d, f)
                    }
                    CellOutcome::Failed { .. } => {
                        let f = fail.fetch_add(1, Ordering::Relaxed) + 1;
                        let d = done.load(Ordering::Relaxed);
                        (d, f)
                    }
                };
                let finished = d + f;
                let pct = if run_total == 0 {
                    100.0_f64
                } else {
                    finished as f64 / run_total as f64 * 100.0
                };
                if is_tty {
                    eprint!("\r[{pct:>5.1}%] done={d} fail={f} / {run_total}   ");
                } else {
                    eprintln!("[{pct:>5.1}%] done={d} fail={f} / {run_total}");
                }
                outcome
            })
            .collect()
    });
    if is_tty && run_total > 0 {
        eprintln!(); // end progress line
    }

    let mut completed = 0usize;
    let mut failed = 0usize;
    for o in &outcomes {
        match o {
            CellOutcome::Done => completed += 1,
            CellOutcome::Failed { error, cell_id } => {
                failed += 1;
                eprintln!("  ✗ {cell_id}: {error}");
            }
        }
    }

    Ok(RunSummary {
        total,
        skipped: skip_count,
        overridden: override_count_pre,
        completed,
        failed,
    })
}

// ── Internal types ────────────────────────────────────────────────────────────

enum CellOutcome {
    Done,
    Failed { cell_id: String, error: String },
}

// ── Cell execution ────────────────────────────────────────────────────────────

fn run_one_cell(
    cell: &MatrixCell,
    prepared: &PreparedProblem,
    registry: &Arc<Mutex<Registry>>,
    sched_ver: &str,
) -> CellOutcome {
    match run_cell_inner(cell, prepared, registry, sched_ver) {
        Ok(()) => CellOutcome::Done,
        Err(error) => CellOutcome::Failed {
            cell_id: cell.cell_id.clone(),
            error,
        },
    }
}

fn run_cell_inner(
    cell: &MatrixCell,
    prepared: &PreparedProblem,
    registry: &Arc<Mutex<Registry>>,
    sched_ver: &str,
) -> Result<(), String> {
    let scheduler_started = Instant::now();
    let schedule = match cell.run_config {
        RunConfig::Est(config) => {
            let scheduler = config.build_scheduler()?;
            scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("EST run {} failed: {e}", cell.cell_id))?
        }
        RunConfig::Hap(config) => {
            let scheduler = config.build_scheduler()?;
            scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("HAP run {} failed: {e}", cell.cell_id))?
        }
        RunConfig::Lst(config) => {
            let scheduler = config.build_scheduler()?;
            scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("LST run {} failed: {e}", cell.cell_id))?
        }
    };
    let scheduler_runtime_ms = scheduler_started.elapsed().as_secs_f64() * 1000.0;

    let metadata = build_schedule_metadata(&cell.run_config, cell, prepared);
    let ctx = MetricsContext::new();
    let mut metrics =
        ScheduleMetrics::compute(&schedule, &prepared.problem, &prepared.horizon, &ctx);
    metrics.scheduler_runtime_ms = Some(scheduler_runtime_ms);

    // The schedules table stores only the invariant body (raw problem +
    // placements). Run-specific metadata and metrics are stored on the run row
    // and recombined at export time.
    let body_obj = ScheduleOutput::new(prepared.raw_json.clone(), &schedule, None);
    let schedule_body_json = serde_json::to_string_pretty(&body_obj)
        .map_err(|e| format!("failed to serialize schedule {}: {e}", cell.cell_id))?;
    let metadata_json = serde_json::to_string(&metadata)
        .map_err(|e| format!("failed to serialize schedule metadata: {e}"))?;
    let metrics_json = serde_json::to_string(&metrics).unwrap_or_else(|_| "{}".to_string());

    let resource_id = prepared.problem.telescope.as_ref().map(|t| t.name.as_str());
    let schedule_hash = canonical_schedule_hash(&schedule, resource_id)?;

    let identity = build_identity(cell, sched_ver)?;
    if let Ok(mut reg) = registry.lock() {
        reg.upsert_result(
            &identity,
            &metrics_json,
            &metadata_json,
            &schedule_hash,
            schedule_body_json.as_str(),
            Some(cell.cell_id.as_str()),
        )?;
    }

    Ok(())
}

/// Re-derives the run-specific schedule metadata for a stored run identity.
///
/// Used by the one-off schedule-deduplication migration to backfill the
/// `runs.metadata_json` column on databases created before that column existed.
/// Metadata is independent of the computed placements, so the scheduler is
/// **not** re-run; only the dataset is loaded to recover the observing site and
/// horizon. Returns an error if the dataset file is missing or its content hash
/// no longer matches the recorded identity.
pub fn metadata_json_from_identity(identity: &RunIdentity) -> Result<String, String> {
    let current_hash = hash_file(std::path::Path::new(&identity.dataset_path))?;
    if current_hash != identity.dataset_hash {
        return Err(format!(
            "dataset hash mismatch for {}: registry has {}, current file has {}",
            identity.dataset_path, identity.dataset_hash, current_hash
        ));
    }
    let run_config: RunConfig = serde_json::from_str(&identity.config_json)
        .map_err(|e| format!("failed to parse run config from identity: {e}"))?;
    let horizon_override = identity
        .horizon_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| format!("failed to parse horizon override from identity: {e}"))?;
    let prepared = prepare_problem(
        std::path::Path::new(&identity.dataset_path),
        horizon_override,
    )?;
    let metadata = build_schedule_metadata_from_parts(
        &run_config,
        &prepared,
        Some(identity.dataset_id.clone()),
        None,
    );
    serde_json::to_string(&metadata).map_err(|e| format!("failed to serialize metadata: {e}"))
}
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
    build_schedule_metadata_from_parts(
        run,
        prepared,
        Some(cell.dataset_id.clone()),
        cell.dataset_label.clone(),
    )
}

fn build_schedule_metadata_from_parts(
    run: &RunConfig,
    prepared: &PreparedProblem,
    dataset_id: Option<String>,
    dataset_label: Option<String>,
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
        dataset_id,
        dataset_label,
    }
}

/// Returns the number of logical CPU cores available.
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
