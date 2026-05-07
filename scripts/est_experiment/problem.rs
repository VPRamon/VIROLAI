use scheduler::prescheduler::{TaskPeriodMap, preschedule};
use scheduler::schedule::SchedulingProblem;
use scheduler::time::{MJD, Period, Time};
use serde_json::Value;
use std::fs;
use std::path::Path;

use super::config::HorizonOverride;

/// A scheduling problem loaded from disk and preprocessed, shared across all runs.
///
/// The prescheduling step (computing feasible windows per task) is expensive and
/// independent of scheduler configuration, so it runs once and is reused.
pub struct PreparedProblem {
    /// Original JSON, preserved for embedding in schedule output files.
    pub raw_json: Value,
    pub problem: SchedulingProblem,
    pub possible_periods: TaskPeriodMap,
    pub horizon: Period<MJD>,
}

/// Loads `input_path`, runs the prescheduler, and returns a [`PreparedProblem`].
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
