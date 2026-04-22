use crate::error::ScheduleError;
use crate::schedule::Schedule;
use crate::scheduling_block::SchedulingBlock;
use crate::time::{MJD, SchedulingBlockId, TaskId, Time};
use std::collections::HashMap;

/// Domain context passed into the beam search when a [`crate::schedule::SchedulingProblem`]
/// is available. Carrying it here allows the EST inner loop to validate
/// dependency ordering rather than bypassing domain invariants with a raw
/// `insert_placement`.
///
/// Hard-constraint coverage is intentionally not re-checked here: the
/// pre-scheduler already guarantees that every window in `possible_periods`
/// is constraint-feasible, and EST only proposes starts within those windows.
pub(super) struct ProblemCtx<'p> {
    /// Pre-computed per-block task lists used for dependency checks.
    pub(super) blocks: &'p HashMap<SchedulingBlockId, SchedulingBlock>,
}

/// Check that all predecessor tasks in the same block are already scheduled
/// and end before `candidate_start`.
///
/// Returns `Ok(())` if the placement is dependency-safe, or a
/// [`ScheduleError`] describing the violation.
pub(super) fn check_block_dependencies(
    schedule: &Schedule,
    task_id: TaskId,
    candidate_start: Time<MJD>,
    block_id: Option<SchedulingBlockId>,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
) -> Result<(), ScheduleError> {
    let Some(block_id) = block_id else {
        return Ok(());
    };
    let Some(block) = blocks.get(&block_id) else {
        return Ok(());
    };
    if !block.contains_task(task_id) {
        return Ok(());
    }

    let order = block.topological_order()?;
    let task_pos = order.iter().position(|&t| t == task_id).unwrap_or(0);

    for &pred_id in order.iter().take(task_pos) {
        match schedule.get(pred_id) {
            None => {
                return Err(ScheduleError::ConstraintViolation(format!(
                    "task {} predecessor {} not yet scheduled",
                    task_id.0, pred_id.0,
                )));
            }
            Some(prev) if prev.end > candidate_start => {
                return Err(ScheduleError::ConstraintViolation(format!(
                    "task {} predecessor {} ends after candidate start",
                    task_id.0, pred_id.0,
                )));
            }
            Some(_) => {}
        }
    }

    Ok(())
}
