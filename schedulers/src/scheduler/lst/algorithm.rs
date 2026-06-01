use super::transform;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduler::SchedulingAlgorithm;
use crate::scheduler::algorithm::validate_task_refs;
use crate::scheduler::cursor::{MultiCursorConfig, run_with_config};
use crate::scheduler::est::{Configuration, ScheduleFom, SoftConstraintFom};
use crate::scheduler::fom::FomContext;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId, Time};
use std::sync::Arc;

/// A FOM wrapper that unmirrored the schedule and context before delegating
/// to an inner figure of merit.
///
/// This type is **no longer used by production code**. `LstScheduler` now
/// delegates directly to the cursor engine via [`run_with_config`]. This
/// struct is retained for backward compatibility with tests that verify the
/// mirroring round-trip. It is not part of the public crate API.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct MirroredFom {
    inner: Arc<dyn ScheduleFom>,
    horizon: Period<MJD>,
}

impl MirroredFom {
    #[allow(dead_code)]
    pub(crate) fn new(inner: Arc<dyn ScheduleFom>, horizon: Period<MJD>) -> Self {
        Self { inner, horizon }
    }
}

impl ScheduleFom for MirroredFom {
    fn evaluate(
        &self,
        schedule: &Schedule,
        problem: &SchedulingProblem,
        ctx: &FomContext<'_>,
    ) -> f64 {
        let original_schedule = transform::unmirror_schedule(schedule, &self.horizon);
        // The mirrored cursor c corresponds to horizon_end - (c - horizon_start) in original
        // space, i.e. the latest time at which a task can *end* in original time.
        let original_horizon_end = transform::mirror_time(ctx.cursor, &self.horizon);

        if let Some(mirrored_periods) = ctx.possible_periods {
            let original_periods =
                transform::unmirror_task_periods(mirrored_periods, &self.horizon);
            let original_ctx = FomContext::single_cursor(
                self.horizon.start,
                Period::new(self.horizon.start, original_horizon_end),
                Some(&original_periods),
            );
            self.inner
                .evaluate(&original_schedule, problem, &original_ctx)
        } else {
            let original_ctx = FomContext::single_cursor(
                self.horizon.start,
                Period::new(self.horizon.start, original_horizon_end),
                None,
            );
            self.inner
                .evaluate(&original_schedule, problem, &original_ctx)
        }
    }

    fn label(&self) -> &'static str {
        "mirrored"
    }
}

/// Latest-Start-Time (LST) scheduler.
///
/// Delegates to the cursor engine configured as a single-backward cursor over
/// the whole horizon. This is the formal definition of LST: it produces
/// exactly the same schedule as
/// [`MultiCursorScheduler::single_backward`](crate::scheduler::cursor::MultiCursorScheduler::single_backward).
///
/// The backward direction is handled inside the shared cursor engine via a
/// `CursorFrame::Mirrored` frame;
/// this scheduler does **not** mirror feasibility windows manually or
/// construct an intermediate [`EstScheduler`](crate::scheduler::est::EstScheduler).
///
/// All tuning parameters (`k_beams`, `branching_factor`,
/// `endangered_threshold`, `fom`) are taken from the shared
/// [`crate::scheduler::est::Configuration`] and [`ScheduleFom`] types so both schedulers can
/// be driven with the same CLI flags.
#[derive(Debug, Clone)]
pub struct LstScheduler {
    pub config: Configuration,
    pub fom: Arc<dyn ScheduleFom>,
}

impl Default for LstScheduler {
    fn default() -> Self {
        Self {
            config: Configuration::default(),
            fom: Arc::new(SoftConstraintFom),
        }
    }
}

impl LstScheduler {
    /// Create an `LstScheduler` with the given config and the default
    /// [`SoftConstraintFom`] figure of merit.
    pub fn new(config: Configuration) -> Result<Self, ScheduleError> {
        Self::from_parts(config, Arc::new(SoftConstraintFom) as Arc<dyn ScheduleFom>)
    }

    /// Create an `LstScheduler` with a custom figure of merit.
    pub fn with_fom(
        config: Configuration,
        fom: Arc<dyn ScheduleFom>,
    ) -> Result<Self, ScheduleError> {
        Self::from_parts(config, fom)
    }

    pub fn from_parts(
        mut config: Configuration,
        fom: Arc<dyn ScheduleFom>,
    ) -> Result<Self, ScheduleError> {
        config.k_beams = config.k_beams.max(1);
        config.branching_factor = config.branching_factor.max(1);
        Ok(Self { config, fom })
    }

    /// Run beam-search LST on a full scheduling problem.
    ///
    /// Delegates to the cursor engine configured as a single-backward cursor
    /// over the whole horizon. This is the formal definition of LST: it
    /// produces exactly the same schedule as
    /// [`MultiCursorScheduler::single_backward`](crate::scheduler::cursor::MultiCursorScheduler::single_backward).
    pub fn run(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "lst: starting scheduler — blocks={}, tasks={}, endangered_threshold={}, k_beams={}, branching_factor={}, horizon=[{:.4}, {:.4}], fom={}",
            problem.block_count(),
            problem.task_count(),
            self.config.endangered_threshold,
            self.config.k_beams,
            self.config.branching_factor,
            horizon.start.value(),
            horizon.end.value(),
            self.fom.label(),
        );

        let config = MultiCursorConfig::single_backward(
            self.config.k_beams,
            self.config.branching_factor,
            self.config.endangered_threshold,
        );
        run_with_config(
            &config,
            self.fom.as_ref(),
            problem,
            possible_periods,
            horizon,
        )
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
        validate_task_refs(tasks.iter())?;
        let blocks = tasks
            .into_iter()
            .map(|task| SchedulingBlock::from_tasks(SchedulingBlockId(task.id.0), vec![task]))
            .collect::<Result<Vec<_>, _>>()?;
        let problem = SchedulingProblem::from_blocks(blocks)?;
        self.run(&problem, possible_periods, horizon)
    }
}

impl SchedulingAlgorithm for LstScheduler {
    fn run_unchecked(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        LstScheduler::run(self, problem, possible_periods, horizon)
    }
}

/// Convenience entry point using the default LST scheduler configuration.
pub fn run_scheduler<I>(
    tasks: I,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError>
where
    I: IntoIterator<Item = Task>,
{
    LstScheduler::default().run_scheduler(tasks, possible_periods, horizon)
}

/// Return the time at which a task is placed in the given schedule.
///
/// Used in tests to extract placement start times by task id.
#[allow(dead_code)]
pub(crate) fn placement_start(schedule: &Schedule, task_id: u64) -> Option<Time<MJD>> {
    schedule.get(crate::time::TaskId(task_id)).map(|p| p.start)
}

/// Return the time at which a task ends in the given schedule.
#[allow(dead_code)]
pub(crate) fn placement_end(schedule: &Schedule, task_id: u64) -> Option<Time<MJD>> {
    schedule.get(crate::time::TaskId(task_id)).map(|p| p.end)
}
