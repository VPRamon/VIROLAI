//! Dataset loading and prescheduling.
//!
//! [`prepare_problem`] reads a scheduling-problem JSON from disk, optionally
//! applies a horizon override, runs the prescheduler once, and bundles the
//! results into a [`PreparedProblem`].
//!
//! The prescheduling step computes the set of feasible time windows per task
//! and is independent of scheduler configuration, so it is performed once and
//! shared across all runs that use the same dataset.

use schedulers::prescheduler::{TaskPeriodMap, preschedule};
use schedulers::schedule::SchedulingProblem;
use schedulers::time::{MJD, Period, Time};
use serde_json::Value;
use std::fs;
use std::path::Path;

use crate::config::HorizonOverride;

// ── Prepared problem ──────────────────────────────────────────────────────────

/// A scheduling problem loaded from disk and preprocessed.
///
/// The prescheduling step (computing feasible windows per task) is expensive
/// and independent of scheduler configuration, so it runs once and is shared
/// across all runs against the same dataset.
pub struct PreparedProblem {
    /// Original raw JSON, preserved for embedding in schedule output files.
    pub raw_json: Value,
    /// Parsed scheduling problem.
    pub problem: SchedulingProblem,
    /// Map of task ID → feasible time windows, produced by the prescheduler.
    pub possible_periods: TaskPeriodMap,
    /// Observing horizon used for this run.
    pub horizon: Period<MJD>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Loads `input_path`, optionally overrides the horizon, runs the prescheduler,
/// and returns a [`PreparedProblem`] ready to be handed to any scheduler.
///
/// # Errors
///
/// Returns an error if the file cannot be read, the JSON is invalid, the
/// problem contains no tasks, the telescope is missing from the input, the
/// horizon is invalid, or the prescheduler fails.
pub fn prepare_problem(
    input_path: &Path,
    horizon_override: Option<HorizonOverride>,
) -> Result<PreparedProblem, String> {
    let text = fs::read_to_string(input_path)
        .map_err(|e| format!("failed to read {}: {e}", input_path.display()))?;
    let raw_json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse JSON {}: {e}", input_path.display()))?;
    let problem: SchedulingProblem = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", input_path.display()))?;

    if problem.task_count() == 0 {
        return Err("input JSON contains no tasks".to_string());
    }

    let horizon = build_horizon(problem.detected_horizon, horizon_override)?;
    let telescope = problem.telescope.as_ref().ok_or_else(|| {
        "missing observing site in input; expected resources[0] or legacy location".to_string()
    })?;
    let possible_periods = preschedule(&problem, &horizon, telescope)
        .map_err(|e| format!("prescheduling failed: {e}"))?;

    Ok(PreparedProblem {
        raw_json,
        problem,
        possible_periods,
        horizon,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Resolves the observing horizon.
///
/// If `override_range` is supplied it takes precedence; otherwise the horizon
/// detected from `schedule_time_window` in the JSON is used.
fn build_horizon(
    detected: Option<Period<MJD>>,
    override_range: Option<HorizonOverride>,
) -> Result<Period<MJD>, String> {
    if let Some(h) = override_range {
        if !h.start_mjd.is_finite() || !h.end_mjd.is_finite() {
            return Err("horizon bounds must be finite numbers".to_string());
        }
        if h.start_mjd >= h.end_mjd {
            return Err(format!(
                "invalid horizon: start ({}) must be before end ({})",
                h.start_mjd, h.end_mjd
            ));
        }
        return Ok(Period::new(
            Time::<MJD>::new(h.start_mjd),
            Time::<MJD>::new(h.end_mjd),
        ));
    }
    detected.ok_or_else(|| {
        "missing schedule_time_window in input and no horizon override was provided".to_string()
    })
}
