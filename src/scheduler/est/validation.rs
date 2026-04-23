use super::algorithm::EstScheduler;
use super::configuration::MAX_K_BEAMS;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::task::Task;
use std::collections::HashSet;

#[inline]
/// Validate EST scheduler configuration before any work is performed.
pub fn validate_scheduler(scheduler: &EstScheduler) -> Result<(), ScheduleError> {
    if scheduler.config.k_beams == 0 {
        return Err(ScheduleError::InvalidConfiguration(
            "est.k_beams must be at least 1".to_string(),
        ));
    }
    if scheduler.config.k_beams > MAX_K_BEAMS {
        // Cap beam width explicitly so malformed configs cannot explode the
        // search-state count.
        return Err(ScheduleError::InvalidConfiguration(format!(
            "est.k_beams must be <= {MAX_K_BEAMS}, got {}",
            scheduler.config.k_beams
        )));
    }
    if scheduler.config.branching_factor == 0 {
        return Err(ScheduleError::InvalidConfiguration(
            "est.branching_factor must be at least 1".to_string(),
        ));
    }
    Ok(())
}

#[inline]
/// Validate the task list consumed by EST.
///
/// Decision checks:
/// 1. reject non-positive durations,
/// 2. reject duplicate task identifiers.
pub fn validate_tasks(tasks: &[Task]) -> Result<(), ScheduleError> {
    validate_task_refs(tasks.iter())
}

pub fn validate_task_refs<'a, I>(tasks: I) -> Result<(), ScheduleError>
where
    I: IntoIterator<Item = &'a Task>,
{
    let tasks: Vec<&Task> = tasks.into_iter().collect();
    log::debug!("est: validating {} task(s)", tasks.len());
    let mut seen_ids = HashSet::with_capacity(tasks.len());

    for task in &tasks {
        if task.duration.value() <= 0.0 {
            log::warn!("est: task {} has non-positive duration", task.id.0);
            return Err(ScheduleError::InvalidDuration);
        }

        // EST depends on unique task ids because schedules and feasible-window
        // maps are keyed by `TaskId`.
        if !seen_ids.insert(task.id) {
            log::warn!("est: duplicate task id {}", task.id.0);
            return Err(ScheduleError::InvalidTask(format!(
                "duplicate task id {}",
                task.id.0
            )));
        }
    }

    log::debug!("est: all {} task(s) are valid", tasks.len());
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
            // EST only builds candidates for tasks with at least one feasible
            // prescheduled period.
            let keep = possible_periods
                .get(&task.id)
                .is_some_and(|periods| !periods.is_empty());
            if !keep {
                log::warn!("est: task {} filtered out (no feasible windows)", task.id.0);
            }
            keep
        })
        .collect();

    let after = filtered.len();
    if before != after {
        log::info!("est: filter_tasks retained {}/{} task(s)", after, before);
    }
    filtered
}
