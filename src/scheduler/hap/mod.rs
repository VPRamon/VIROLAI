//! HAP / AP — Accumulative Planner family.
//!
//! Two top-level entry points configured around a single accumulative core:
//!
//! - [`ap::run`] — deterministic greedy, single output schedule.
//! - [`hap::run`] — stochastic multi-start, set of output schedules.
//!
//! Both reuse the [`cru`] module (Conflict Resolution Unit + variants) for
//! per-block candidate generation and the shared [`eval`] / [`selection`]
//! helpers for fitness and survivor selection.

pub mod accumulative;
pub mod ap;
pub mod configuration;
pub mod cru;
pub mod eval;
#[allow(clippy::module_inception)]
pub mod hap;
pub mod selection;

pub use configuration::{Configuration, PlannerConfig, Selector, SurvivorSelector};

use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduler::SchedulingAlgorithm;
use crate::time::{MJD, Period};

/// Public HAP scheduler adapter around the active accumulative-planner API.
#[derive(Debug, Clone)]
pub struct HapScheduler {
    pub config: PlannerConfig,
}

impl Default for HapScheduler {
    fn default() -> Self {
        Self {
            config: default_planner_config(),
        }
    }
}

impl HapScheduler {
    pub fn new(config: PlannerConfig) -> Self {
        Self { config }
    }

    pub fn run(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "hap: starting scheduler — blocks={}, tasks={}, population_size={}, max_iter={}, stochastic_range={}, horizon=[{:.4}, {:.4}]",
            problem.block_count(),
            problem.task_count(),
            self.config.population_size,
            self.config.cru.max_iter,
            self.config.cru.stochastic_range,
            horizon.start.value(),
            horizon.end.value(),
        );

        let survivors = hap::run_with_config(
            &Schedule::new(),
            problem,
            possible_periods,
            horizon,
            &self.config,
        );
        let mut selected = selection::select(
            SurvivorSelector::GreedyOne,
            survivors,
            problem,
            horizon.start,
        );
        Ok(selected.pop().unwrap_or_default())
    }
}

impl SchedulingAlgorithm for HapScheduler {
    fn run_unchecked(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        HapScheduler::run(self, problem, possible_periods, horizon)
    }
}

pub fn default_planner_config() -> PlannerConfig {
    PlannerConfig::hap(128, 3, 4, SurvivorSelector::ElitistTopK { k: 4 }, 0)
}
