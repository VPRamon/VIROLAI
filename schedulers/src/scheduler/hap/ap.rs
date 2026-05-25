//! Accumulative Planner (AP) entry point.
//!
//! AP is the deterministic, greedy variant of the accumulative planner: it
//! maintains a single schedule, sorts blocks by descending priority,
//! attempts each block once via deterministic CRU, and keeps the highest
//! fitness candidate (which may be the unchanged input schedule when CRU
//! cannot improve fitness — the "rejection" candidate).

use super::accumulative::accumulative_plan;
use super::configuration::PlannerConfig;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::time::{MJD, Period};

/// Run AP and return the single resulting schedule.
///
/// `iota_max` is the per-block CRU inner-cycle limit (`ι_max`).
pub fn run(
    input: &Schedule,
    problem: &SchedulingProblem,
    periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    iota_max: usize,
) -> Schedule {
    let cfg = PlannerConfig::ap(iota_max);
    let mut survivors = accumulative_plan(input, problem, periods, horizon, &cfg);
    survivors.pop().unwrap_or_else(|| input.clone())
}

/// Run AP using a fully-customised [`PlannerConfig`]. Useful when a caller
/// wants AP-style control flow with a non-default CRU `ι_max` or rejection
/// behavior.
pub fn run_with_config(
    input: &Schedule,
    problem: &SchedulingProblem,
    periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    cfg: &PlannerConfig,
) -> Schedule {
    let mut survivors = accumulative_plan(input, problem, periods, horizon, cfg);
    survivors.pop().unwrap_or_else(|| input.clone())
}
