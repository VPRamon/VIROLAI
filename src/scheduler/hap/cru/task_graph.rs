//! Task dependency helpers for CRU.

use crate::schedule::Schedule;
use crate::scheduling_block::SchedulingBlock;
use crate::time::{MJD, Period, SchedulingBlockId, TaskId, Time};
use std::collections::HashMap;

/// Returns `true` when all direct predecessors of `task_id` (within its block)
/// are already placed in `schedule`.
pub(super) fn predecessors_placed(
    task_id: TaskId,
    schedule: &Schedule,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
) -> bool {
    let Some(&block_id) = task_to_block.get(&task_id) else {
        return true;
    };
    let Some(block) = blocks.get(&block_id) else {
        return true;
    };
    block
        .predecessors(task_id)
        .iter()
        .all(|pred_id| schedule.contains(*pred_id))
}

/// Returns the maximum end time of all placed predecessors of `task_id`,
/// falling back to `horizon.start` when no predecessors are placed.
pub(super) fn predecessor_end_lower_bound(
    task_id: TaskId,
    schedule: &Schedule,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    horizon: &Period<MJD>,
) -> Time<MJD> {
    let Some(&block_id) = task_to_block.get(&task_id) else {
        return horizon.start;
    };
    let Some(block) = blocks.get(&block_id) else {
        return horizon.start;
    };
    block
        .predecessors(task_id)
        .iter()
        .filter_map(|pred_id| schedule.get(*pred_id).map(|p| p.end))
        .max_by(|a, b| {
            a.value()
                .partial_cmp(&b.value())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(horizon.start)
}
