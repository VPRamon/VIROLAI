use super::beam;
use super::configuration::Configuration;
use super::context::ProblemCtx;
use super::queue::CandidateQueue;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduler::fom::{ScheduleFom, SoftConstraintFom};
use crate::scheduler::{SchedulingAlgorithm, filter_task_refs};
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId};
use std::marker::PhantomData;

/// EST scheduler implementation.
#[derive(Debug, Clone)]
pub struct EstScheduler<F: ScheduleFom> {
    /// Search parameters controlling endangered detection, beam width, and branching.
    pub config: Configuration,
    /// Figure of merit used to rank and prune beam states after each round.
    pub fom: F,
    _phantom: PhantomData<F>,
}

impl Default for EstScheduler<SoftConstraintFom> {
    /// Construct the default single-beam EST scheduler scored by soft constraints.
    fn default() -> Self {
        Self {
            config: Configuration::default(),
            fom: SoftConstraintFom,
            _phantom: PhantomData,
        }
    }
}

impl EstScheduler<SoftConstraintFom> {
    /// Create an `EstScheduler` with the given config and the default
    /// [`SoftConstraintFom`] figure of merit.
    pub fn new(config: Configuration) -> Result<Self, ScheduleError> {
        Self::from_parts(config, SoftConstraintFom)
    }
}

impl<F: ScheduleFom> EstScheduler<F> {
    fn from_parts(mut config: Configuration, fom: F) -> Result<Self, ScheduleError> {
        config.k_beams = config.k_beams.max(1);
        config.branching_factor = config.branching_factor.max(1);

        let scheduler = Self {
            config,
            fom,
            _phantom: PhantomData,
        };
        Ok(scheduler)
    }

    /// Backward-compatible constructor for callers that already have a FOM.
    pub fn with_fom(config: Configuration, fom: F) -> Result<Self, ScheduleError> {
        Self::from_parts(config, fom)
    }

    /// Preserve the legacy API used by the experiment runners.
    pub fn with_fom_label(self, _label: String) -> Self {
        self
    }

    /// Run beam-search EST on a full scheduling problem.
    pub fn run(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "est: starting scheduler — blocks={}, tasks={}, endangered_threshold={}, k_beams={}, branching_factor={}, horizon=[{:.4}, {:.4}], fom={}",
            problem.block_count(),
            problem.task_count(),
            self.config.endangered_threshold,
            self.config.k_beams,
            self.config.branching_factor,
            horizon.start.value(),
            horizon.end.value(),
            self.fom.label(),
        );

        let filtered_tasks = filter_task_refs(problem.iter_tasks(), possible_periods);

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

        Ok(beam::run_search(
            self,
            initial_state,
            horizon,
            problem,
            Some(&ProblemCtx {
                problem,
                possible_periods,
            }),
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
        crate::scheduler::algorithm::validate_task_refs(tasks.iter())?;
        let blocks = tasks
            .into_iter()
            .map(|task| SchedulingBlock::from_tasks(SchedulingBlockId(task.id.0), vec![task]))
            .collect::<Result<Vec<_>, _>>()?;
        let problem = SchedulingProblem::from_blocks(blocks)?;
        self.run(&problem, possible_periods, horizon)
    }
}

/// Convenience entry point using the default EST scheduler configuration.
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

impl<F: ScheduleFom> SchedulingAlgorithm for EstScheduler<F> {
    fn run_unchecked(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        EstScheduler::run(self, problem, possible_periods, horizon)
    }
}
