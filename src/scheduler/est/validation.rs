use super::algorithm::EstScheduler;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::task::Task;
use std::collections::HashSet;

#[inline]
pub fn validate_scheduler(_scheduler: &EstScheduler) -> Result<(), ScheduleError> {
    Ok(())
}

#[inline]
pub fn validate_tasks(tasks: &[Task]) -> Result<(), ScheduleError> {
    log::debug!("est: validating {} task(s)", tasks.len());
    let mut seen_ids = HashSet::with_capacity(tasks.len());

    for task in tasks {
        if task.duration.value() <= 0.0 {
            log::warn!("est: task {} has non-positive duration", task.id.0);
            return Err(ScheduleError::InvalidDuration);
        }

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

#[inline]
pub fn filter_tasks(tasks: Vec<Task>, possible_periods: &TaskPeriodMap) -> Vec<Task> {
    let before = tasks.len();
    let filtered: Vec<Task> = tasks
        .into_iter()
        .filter(|task| {
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
