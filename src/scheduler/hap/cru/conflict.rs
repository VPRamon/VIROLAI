//! Conflict-group and conflict-cost helpers for CRU.

use super::super::block_eval::{
    block_impatience_denominator, block_is_complete, block_priority, block_task_count,
    min_positive_block_priority,
};
use super::super::configuration::Configuration;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, TaskId, Time};
use qtty::Day;
use std::collections::HashSet;

pub(super) fn compute_min_positive_priority(
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> f64 {
    min_positive_block_priority(problem, horizon_start)
}

/// Compute the conflict group for placing `task` starting at `start`:
/// - All placed tasks overlapping `[start, start + duration)`.
/// - Closed over dependency descendants in the same block that are also placed.
pub(super) fn compute_conflict_group(
    start: Time<MJD>,
    task: &Task,
    schedule: &Schedule,
    problem: &SchedulingProblem,
) -> HashSet<TaskId> {
    let duration_days = task.duration.to::<Day>().value();
    let end = Time::<MJD>::new(start.value() + duration_days);
    let interval = Period::new(start, end);

    let initial: HashSet<TaskId> = schedule.overlapping(&interval).into_iter().collect();
    let mut closed = initial.clone();

    for &base_id in &initial {
        if let Some(block_id) = problem.task_block_id(base_id)
            && let Some(block) = problem.block(block_id)
        {
            for desc_id in block.all_descendants(base_id) {
                if schedule.contains(desc_id) {
                    closed.insert(desc_id);
                }
            }
        }
    }

    closed
}

/// Compute the total conflict cost of placing `task` at `start`.
///
/// Cost = sum over conflicting tasks of:
/// - `conflicting_block.priority + impatience * (min_positive_priority / alpha)`
///   when the conflicting task belongs to a currently-complete scheduling block.
/// - `0.0` otherwise.
#[allow(clippy::too_many_arguments)]
pub(super) fn compute_conflict_cost(
    start: Time<MJD>,
    task: &Task,
    schedule: &Schedule,
    problem: &SchedulingProblem,
    current_block: &SchedulingBlock,
    possible_periods: &TaskPeriodMap,
    horizon_start: Time<MJD>,
    min_positive_priority: f64,
    config: &Configuration,
) -> f64 {
    let impatience = block_task_count(current_block) as f64
        / block_impatience_denominator(current_block, possible_periods) as f64;
    let conflict_group = compute_conflict_group(start, task, schedule, problem);

    let mut total_cost = 0.0;
    for conflicting_id in &conflict_group {
        let Some(conflicting_block_id) = problem.task_block_id(*conflicting_id) else {
            continue;
        };
        let Some(conflicting_block) = problem.block(conflicting_block_id) else {
            continue;
        };
        if block_is_complete(conflicting_block, schedule) {
            total_cost += block_priority(conflicting_block, problem, horizon_start)
                + impatience * (min_positive_priority / config.impatience_alpha);
        }
    }
    total_cost
}
