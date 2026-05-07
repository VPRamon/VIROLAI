//! Matrix runner: prepare datasets, execute cells (in parallel with
//! checkpointing), and collect a summary CSV.

use chrono::Utc;
use rayon::prelude::*;
use scheduler::metrics::{MetricsContext, RankingWeights, ScheduleMetrics};
use scheduler::schedule::{LocationMeta, PeriodMeta, ScheduleMetadata, ScheduleOutput};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cell::MatrixCell;
use crate::est_experiment::config::{HapSurvivorMode, RunConfig};
use crate::est_experiment::problem::{PreparedProblem, prepare_problem};
use crate::output;
use crate::output::SummaryRow;
use crate::spec::ExperimentSpec;
use crate::state::{CellStatus, StateEvent, StateWriter, completed_cells, read_events};

/// Entry point for `run` and `--resume`.
pub fn execute(
    spec: &ExperimentSpec,
    cells: &[MatrixCell],
    run_dir: &Path,
    resume: bool,
) -> Result<RunSummary, String> {
    output::init_subdirs(run_dir)?;

    // Skip cells already marked completed (only on resume).
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
        "experiment_matrix: {} total cells, {} already completed, {} to run",
        cells.len(),
        already_done.len(),
        pending.len()
    );

    // Prepare each unique dataset once.
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

    // Concurrency: bounded rayon pool.
    let max_parallel = spec
        .max_parallel
        .map(|n| n.max(1))
        .unwrap_or_else(|| std::cmp::max(1, std::cmp::min(num_cpus_guess(), pending.len().max(1))));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(max_parallel)
        .build()
        .map_err(|e| format!("failed to build rayon pool: {e}"))?;

    let state_writer = Arc::new(StateWriter::open_append(&run_dir.join(output::STATE_FILE))?);
    let ranking: Option<RankingWeights> = spec.ranking.map(Into::into);
    let emit_trace = spec.emit_trace;
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
                    emit_trace,
                    ranking,
                    &state_writer,
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

    // Build summary.csv from every on-disk metric file (so resumed runs include
    // historical cells too).
    let summary_rows = collect_summary_rows(cells, run_dir)?;
    output::write_summary_csv(&run_dir.join(output::SUMMARY_FILE), &summary_rows)?;

    Ok(RunSummary {
        total: cells.len(),
        already_done: already_done.len(),
        completed,
        failed,
        run_dir: run_dir.to_path_buf(),
    })
}

pub struct RunSummary {
    pub total: usize,
    pub already_done: usize,
    pub completed: usize,
    pub failed: usize,
    pub run_dir: PathBuf,
}

#[allow(dead_code)]
enum CellOutcome {
    Done { cell_id: String },
    Failed { cell_id: String, error: String },
}

fn run_one_cell(
    cell: &MatrixCell,
    prepared: &PreparedProblem,
    run_dir: &Path,
    emit_trace: bool,
    ranking: Option<RankingWeights>,
    state_writer: &StateWriter,
) -> CellOutcome {
    let started_at = Utc::now().to_rfc3339();
    let _ = state_writer.append(&StateEvent {
        cell_id: cell.cell_id.clone(),
        status: CellStatus::Started,
        schedule_path: None,
        metrics_path: None,
        trace_path: None,
        error: None,
        started_at: started_at.clone(),
        finished_at: None,
    });

    match run_cell_inner(cell, prepared, run_dir, emit_trace, ranking) {
        Ok(paths) => {
            let _ = state_writer.append(&StateEvent {
                cell_id: cell.cell_id.clone(),
                status: CellStatus::Completed,
                schedule_path: Some(paths.schedule_path.display().to_string()),
                metrics_path: Some(paths.metrics_path.display().to_string()),
                trace_path: paths.trace_path.as_ref().map(|p| p.display().to_string()),
                error: None,
                started_at,
                finished_at: Some(Utc::now().to_rfc3339()),
            });
            CellOutcome::Done {
                cell_id: cell.cell_id.clone(),
            }
        }
        Err(error) => {
            let _ = state_writer.append(&StateEvent {
                cell_id: cell.cell_id.clone(),
                status: CellStatus::Failed,
                schedule_path: None,
                metrics_path: None,
                trace_path: None,
                error: Some(error.clone()),
                started_at,
                finished_at: Some(Utc::now().to_rfc3339()),
            });
            CellOutcome::Failed {
                cell_id: cell.cell_id.clone(),
                error,
            }
        }
    }
}

struct CellPaths {
    schedule_path: PathBuf,
    metrics_path: PathBuf,
    trace_path: Option<PathBuf>,
}

fn run_cell_inner(
    cell: &MatrixCell,
    prepared: &PreparedProblem,
    run_dir: &Path,
    emit_trace: bool,
    ranking: Option<RankingWeights>,
) -> Result<CellPaths, String> {
    let schedule_path = output::schedule_path(run_dir, &cell.cell_id);
    let metrics_path = output::metrics_path(run_dir, &cell.cell_id);

    let (schedule, trace_path) = match cell.run_config {
        RunConfig::Est(config) => {
            let mut scheduler = config.build_scheduler()?;
            scheduler = scheduler.with_fom_label(config.fom.to_string());
            let schedule = scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("EST run {} failed: {e}", cell.cell_id))?;
            let _ = emit_trace;
            (schedule, None)
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
            (schedule, None)
        }
    };

    let metadata = build_schedule_metadata(&cell.run_config, prepared);
    let output_obj = ScheduleOutput::new(prepared.raw_json.clone(), &schedule, Some(metadata));
    let text = serde_json::to_string_pretty(&output_obj)
        .map_err(|e| format!("failed to serialize schedule {}: {e}", cell.cell_id))?;
    fs::write(&schedule_path, text)
        .map_err(|e| format!("failed to write {}: {e}", schedule_path.display()))?;

    let mut ctx = MetricsContext::new();
    if let Some(r) = ranking {
        ctx = ctx.with_ranking(r);
    }
    let metrics = ScheduleMetrics::compute(&schedule, &prepared.problem, &prepared.horizon, &ctx);
    let m_text = serde_json::to_string_pretty(&metrics)
        .map_err(|e| format!("failed to serialize metrics {}: {e}", cell.cell_id))?;
    fs::write(&metrics_path, m_text)
        .map_err(|e| format!("failed to write {}: {e}", metrics_path.display()))?;

    Ok(CellPaths {
        schedule_path,
        metrics_path,
        trace_path,
    })
}

fn collect_summary_rows(cells: &[MatrixCell], run_dir: &Path) -> Result<Vec<SummaryRow>, String> {
    let mut rows = Vec::new();
    for cell in cells {
        let p = output::metrics_path(run_dir, &cell.cell_id);
        if !p.exists() {
            continue;
        }
        let text =
            fs::read_to_string(&p).map_err(|e| format!("failed to read {}: {e}", p.display()))?;
        let metrics: ScheduleMetrics = serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse {}: {e}", p.display()))?;
        rows.push(SummaryRow::from_metrics(cell, &metrics));
    }
    Ok(rows)
}

fn build_schedule_metadata(run: &RunConfig, prepared: &PreparedProblem) -> ScheduleMetadata {
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
    }
}

fn num_cpus_guess() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
