use super::beam;
use super::configuration::Configuration;
use super::context::ProblemCtx;
use super::fom::{ScheduleFom, ScoringContext, SoftConstraintFom};
use super::queue::CandidateQueue;
use super::validation;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId, TaskId};
use std::collections::HashMap;
use std::sync::Arc;

/// EST scheduler implementation.
#[derive(Debug, Clone)]
pub struct EstScheduler {
    /// Search parameters controlling endangered detection, beam width, and branching.
    pub config: Configuration,
    /// Figure of merit used to rank and prune beam states after each round.
    pub fom: Arc<dyn ScheduleFom>,
}

impl Default for EstScheduler {
    /// Construct the default single-beam EST scheduler scored by soft constraints.
    fn default() -> Self {
        Self {
            config: Configuration::default(),
            fom: Arc::new(SoftConstraintFom),
        }
    }
}

impl EstScheduler {
    fn from_parts(config: Configuration, fom: Arc<dyn ScheduleFom>) -> Result<Self, ScheduleError> {
        let scheduler = Self { config, fom };
        validation::validate_scheduler(&scheduler)?;
        Ok(scheduler)
    }

    /// Create an `EstScheduler` with the given config and the default
    /// [`SoftConstraintFom`] figure of merit.
    pub fn new(config: Configuration) -> Result<Self, ScheduleError> {
        Self::from_parts(config, Arc::new(SoftConstraintFom))
    }

    /// Create an `EstScheduler` with a custom figure of merit.
    pub fn with_fom(
        config: Configuration,
        fom: Arc<dyn ScheduleFom>,
    ) -> Result<Self, ScheduleError> {
        Self::from_parts(config, fom)
    }

    /// Run beam-search EST on `tasks` using the provided feasible windows.
    ///
    /// Each round every live beam is expanded by placing up to
    /// `branching_factor` distinct candidates. The resulting child states are
    /// evaluated with the configured FOM and the top `k_beams` survivors are
    /// carried into the next round. The best terminal state is returned.
    pub fn run_scheduler(
        &self,
        tasks: &[Task],
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "est: starting scheduler — tasks={}, endangered_threshold={}, k_beams={}, branching_factor={}, horizon=[{:.4}, {:.4}]",
            tasks.len(),
            self.config.endangered_threshold,
            self.config.k_beams,
            self.config.branching_factor,
            horizon.start.value(),
            horizon.end.value(),
        );

        validation::validate_tasks(tasks)?;
        let filtered_tasks = validation::filter_tasks(tasks, possible_periods);

        log::debug!(
            "est: {} tasks remain after feasibility filter",
            filtered_tasks.len()
        );

        let initial_candidates = CandidateQueue::build(
            &filtered_tasks,
            possible_periods,
            horizon,
            None,
            self.config.endangered_threshold,
        );

        let initial_state = super::ScheduleState {
            cursor: horizon.start,
            schedule: Schedule::new(),
            candidates: initial_candidates,
            score: 0.0,
        };

        let scoring_ctx = ScoringContext::new(tasks);
        Ok(beam::run_search(
            self,
            initial_state,
            horizon,
            &scoring_ctx,
            None,
        ))
    }

    /// Run beam-search EST through the domain model.
    ///
    /// Behaves like [`Self::run_scheduler`] but routes every placement through
    /// dependency-aware domain checks before it is committed.
    pub fn run_with_problem(
        &self,
        tasks: &[Task],
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
        blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "est: starting domain-aware scheduler — tasks={}, endangered_threshold={}, k_beams={}, branching_factor={}, horizon=[{:.4}, {:.4}]",
            tasks.len(),
            self.config.endangered_threshold,
            self.config.k_beams,
            self.config.branching_factor,
            horizon.start.value(),
            horizon.end.value(),
        );

        validation::validate_tasks(tasks)?;
        let filtered_tasks = validation::filter_tasks(tasks, possible_periods);

        log::debug!(
            "est: {} tasks remain after feasibility filter",
            filtered_tasks.len()
        );

        // Build task→block map so candidates carry their block affiliation.
        let task_block_map: HashMap<TaskId, SchedulingBlockId> = blocks
            .iter()
            .flat_map(|(&block_id, block)| block.iter().map(move |task_id| (task_id, block_id)))
            .collect();

        let ctx = ProblemCtx { blocks };

        let initial_candidates = CandidateQueue::build(
            &filtered_tasks,
            possible_periods,
            horizon,
            Some(&task_block_map),
            self.config.endangered_threshold,
        );

        let initial_state = super::ScheduleState {
            cursor: horizon.start,
            schedule: Schedule::new(),
            candidates: initial_candidates,
            score: 0.0,
        };

        let scoring_ctx = ScoringContext::new(tasks);
        Ok(beam::run_search(
            self,
            initial_state,
            horizon,
            &scoring_ctx,
            Some(&ctx),
        ))
    }
}

/// Convenience entry point for the default single-beam, task-count EST run.
pub fn run_scheduler(
    tasks: &[Task],
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError> {
    EstScheduler::default().run_scheduler(tasks, possible_periods, horizon)
}
