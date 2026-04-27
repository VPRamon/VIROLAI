//! Hybrid Accumulative Planner (HAP) entry point.
//!
//! HAP is the stochastic multi-start variant of the accumulative planner:
//! it maintains a set of schedules, sorts blocks by descending priority,
//! and per block runs `population_size` CRU-S attempts (each seeded from a
//! source schedule pulled round-robin from the current set). Surviving
//! schedules are selected via the configured
//! [`SurvivorSelector`](super::configuration::SurvivorSelector) — typically
//! [`ElitistTopK`](super::configuration::SurvivorSelector::ElitistTopK) or
//! [`ParetoFront`](super::configuration::SurvivorSelector::ParetoFront).

use super::accumulative::accumulative_plan;
use super::configuration::{PlannerConfig, SurvivorSelector};
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::time::{MJD, Period};

/// Run HAP with the given preset and return the surviving set of
/// schedules. See [`PlannerConfig::hap`].
#[allow(clippy::too_many_arguments)]
pub fn run(
    input: &Schedule,
    problem: &SchedulingProblem,
    periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    iota_max: usize,
    rho: usize,
    population_size: usize,
    survivor: SurvivorSelector,
    seed: u64,
) -> Vec<Schedule> {
    let cfg = PlannerConfig::hap(iota_max, rho, population_size, survivor, seed);
    accumulative_plan(input, problem, periods, horizon, &cfg)
}

/// Run HAP using a fully-customised [`PlannerConfig`].
pub fn run_with_config(
    input: &Schedule,
    problem: &SchedulingProblem,
    periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    cfg: &PlannerConfig,
) -> Vec<Schedule> {
    accumulative_plan(input, problem, periods, horizon, cfg)
}
