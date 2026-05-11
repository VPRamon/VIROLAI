//! Single-cell scheduler execution.
//!
//! [`execute_run`] takes a [`RunConfig`] and a [`PreparedProblem`], runs the
//! appropriate scheduler, serialises the resulting schedule to disk, and
//! returns a [`RunOutcome`] containing the computed metrics.
//!
//! This module is used by both the legacy single-dataset sweep path and the
//! matrix runner ([`crate::runner`]).

use scheduler::metrics::{MetricsContext, ScheduleMetrics};
use scheduler::schedule::{LocationMeta, PeriodMeta, ScheduleMetadata, ScheduleOutput};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{HapSurvivorMode, RunConfig};
use crate::problem::PreparedProblem;

/// The result of a single scheduler run.
pub struct RunOutcome {
    /// The configuration that was executed.
    pub config: RunConfig,
    /// Path to the schedule JSON that was written.
    pub schedule_path: PathBuf,
    /// Path to the trace JSONL file, if one was produced.
    pub trace_path: Option<PathBuf>,
    /// Computed metrics for the produced schedule.
    pub metrics: ScheduleMetrics,
}

/// Runs the scheduler for `run`, writes the schedule JSON to `schedule_path`,
/// and returns a [`RunOutcome`] with metrics.
///
/// `trace_path` is accepted for future EST trace output but is currently
/// unused.
pub fn execute_run(
    run: &RunConfig,
    prepared: &PreparedProblem,
    schedule_path: &Path,
    trace_path: Option<&Path>,
) -> Result<RunOutcome, String> {
    let (schedule, trace_path_owned) = match *run {
        RunConfig::Est(config) => {
            let mut scheduler = config.build_scheduler()?;
            scheduler = scheduler.with_fom_label(config.fom.to_string());

            let schedule = scheduler
                .run(
                    &prepared.problem,
                    &prepared.possible_periods,
                    &prepared.horizon,
                )
                .map_err(|e| format!("EST run {} failed: {e}", run.slug()))?;
            let _ = trace_path;
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
                .map_err(|e| format!("HAP run {} failed: {e}", run.slug()))?;
            (schedule, None)
        }
    };

    let metadata = build_schedule_metadata(run, prepared);
    let output = ScheduleOutput::new(prepared.raw_json.clone(), &schedule, Some(metadata));
    let output_text = serde_json::to_string_pretty(&output)
        .map_err(|e| format!("failed to serialize schedule output {}: {e}", run.slug()))?;
    fs::write(schedule_path, output_text).map_err(|e| {
        format!(
            "failed to write schedule output {}: {e}",
            schedule_path.display()
        )
    })?;

    let metrics = ScheduleMetrics::compute(
        &schedule,
        &prepared.problem,
        &prepared.horizon,
        &MetricsContext::default(),
    );

    Ok(RunOutcome {
        config: *run,
        schedule_path: schedule_path.to_path_buf(),
        trace_path: trace_path_owned,
        metrics,
    })
}

/// Builds the [`ScheduleMetadata`] embedded in every output schedule JSON.
pub fn build_schedule_metadata(run: &RunConfig, prepared: &PreparedProblem) -> ScheduleMetadata {
    let location = prepared
        .problem
        .telescope
        .as_ref()
        .map(|telescope| LocationMeta {
            name: telescope.name.clone(),
            longitude_deg: telescope.location.lon.value(),
            latitude_deg: telescope.location.lat.value(),
            height_m: telescope.location.height.value(),
        });

    let period = Some(PeriodMeta {
        start_mjd_utc: prepared.horizon.start.value(),
        end_mjd_utc: prepared.horizon.end.value(),
    });

    let algorithm_config = match *run {
        RunConfig::Est(config) => serde_json::json!({
            "k_beams": config.k_beams,
            "branching_factor": config.branching_factor,
            "endangered_threshold": config.endangered_threshold,
            "fom": config.fom.to_string(),
        }),
        RunConfig::Hap(config) => {
            let survivor = match config.survivor_mode {
                HapSurvivorMode::GreedyOne => serde_json::json!({
                    "mode": config.survivor_mode.to_string()
                }),
                HapSurvivorMode::ElitistTopK | HapSurvivorMode::ParetoFront => {
                    serde_json::json!({
                        "mode": config.survivor_mode.to_string(),
                        "cap": config.survivor_cap,
                    })
                }
            };
            serde_json::json!({
                "iota_max": config.iota_max,
                "rho": config.rho,
                "population_size": config.population_size,
                "survivor": survivor,
                "seed": config.seed,
            })
        }
    };

    ScheduleMetadata {
        algorithm: run.algorithm().to_string(),
        algorithm_config,
        location,
        period,
        dataset_id: None,
        dataset_label: None,
    }
}
