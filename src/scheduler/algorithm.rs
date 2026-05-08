use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::task::Task;
use crate::time::{MJD, Period};
use std::collections::HashSet;

pub(crate) fn validate_task_refs<'a, I>(tasks: I) -> Result<(), ScheduleError>
where
    I: IntoIterator<Item = &'a Task>,
{
    let mut seen_ids = HashSet::new();

    for task in tasks {
        if task.duration.value() <= 0.0 {
            return Err(ScheduleError::InvalidDuration);
        }
        if !seen_ids.insert(task.id) {
            return Err(ScheduleError::InvalidTask(format!(
                "duplicate task id {}",
                task.id.0
            )));
        }
    }

    Ok(())
}

pub fn filter_task_refs<'a, I>(tasks: I, possible_periods: &TaskPeriodMap) -> Vec<&'a Task>
where
    I: IntoIterator<Item = &'a Task>,
{
    let tasks: Vec<&Task> = tasks.into_iter().collect();
    let before = tasks.len();
    let filtered: Vec<&Task> = tasks
        .into_iter()
        .filter(|task| {
            let keep = possible_periods
                .get(&task.id)
                .is_some_and(|periods| !periods.is_empty());
            if !keep {
                log::warn!("task {} filtered out (no feasible windows)", task.id.0);
            }
            keep
        })
        .collect();

    let after = filtered.len();
    if before != after {
        log::info!("filter_tasks retained {}/{} task(s)", after, before);
    }
    filtered
}

/// Common interface for scheduling algorithms that operate on a pre-scheduled problem.
pub trait SchedulingAlgorithm {
    fn run_unchecked(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError>;

    fn run(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        validate_task_refs(problem.iter_tasks())?;
        self.run_unchecked(problem, possible_periods, horizon)
    }
}
