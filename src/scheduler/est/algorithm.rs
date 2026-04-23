use super::beam;
use super::configuration::Configuration;
use super::context::ProblemCtx;
use super::fom::{ScheduleFom, ScoringContext, SoftConstraintFom};
use super::queue::CandidateQueue;
use super::validation;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId};
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

    /// Run beam-search EST on a full scheduling problem.
    pub fn run(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "est: starting scheduler — blocks={}, tasks={}, endangered_threshold={}, k_beams={}, branching_factor={}, horizon=[{:.4}, {:.4}]",
            problem.block_count(),
            problem.task_count(),
            self.config.endangered_threshold,
            self.config.k_beams,
            self.config.branching_factor,
            horizon.start.value(),
            horizon.end.value(),
        );

        validation::validate_task_refs(problem.iter_tasks())?;
        let filtered_tasks = validation::filter_task_refs(problem.iter_tasks(), possible_periods);

        log::debug!(
            "est: {} tasks remain after feasibility filter",
            filtered_tasks.len()
        );

        let initial_candidates = CandidateQueue::build(
            &filtered_tasks,
            possible_periods,
            horizon,
            self.config.endangered_threshold,
        );

        let initial_state = super::ScheduleState {
            cursor: horizon.start,
            schedule: Schedule::new(),
            candidates: initial_candidates,
            score: 0.0,
        };

        let scoring_ctx = ScoringContext::new(problem);
        Ok(beam::run_search(
            self,
            initial_state,
            horizon,
            &scoring_ctx,
            Some(&ProblemCtx { problem }),
        ))
    }

    /// Convenience entry point that wraps flat tasks into singleton blocks.
    pub fn run_scheduler<I>(
        &self,
        tasks: I,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError>
    where
        I: IntoIterator<Item = Task>,
    {
        let tasks: Vec<Task> = tasks.into_iter().collect();
        validation::validate_tasks(&tasks)?;
        let blocks = tasks
            .into_iter()
            .map(|task| SchedulingBlock::from_tasks(SchedulingBlockId(task.id.0), vec![task]))
            .collect::<Result<Vec<_>, _>>()?;
        let problem = SchedulingProblem::from_blocks(blocks)?;
        self.run(&problem, possible_periods, horizon)
    }
}

/// Convenience entry point for the default single-beam, task-count EST run.
pub fn run_scheduler<I>(
    tasks: I,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError>
where
    I: IntoIterator<Item = Task>,
{
    EstScheduler::default().run_scheduler(tasks, possible_periods, horizon)
}
