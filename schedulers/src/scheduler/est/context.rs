use crate::error::ScheduleError;
use crate::schedule::Schedule;
use crate::schedule::SchedulingProblem;
use crate::time::{MJD, TaskId, Time};

/// Check that all predecessor tasks in the same block are already scheduled
/// and end before `candidate_start`.
///
/// Returns `Ok(())` if the placement is dependency-safe, or a
/// [`ScheduleError`] describing the violation.
pub(crate) fn check_block_dependencies(
    schedule: &Schedule,
    task_id: TaskId,
    candidate_start: Time<MJD>,
    problem: &SchedulingProblem,
) -> Result<(), ScheduleError> {
    let Some(block_id) = problem.task_block_id(task_id) else {
        return Ok(());
    };
    let Some(block) = problem.block(block_id) else {
        return Ok(());
    };

    let mut predecessors: Vec<_> = block.all_predecessors(task_id).into_iter().collect();
    predecessors.sort_by_key(|task_id| task_id.0);

    for pred_id in predecessors {
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
