//! Conflict-group and conflict-cost helpers for CRU.

use super::super::configuration::Configuration;
use super::super::proposal::Proposal;
use crate::schedule::Schedule;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId, TaskId, Time};
use qtty::Day;
use std::collections::{HashMap, HashSet};

pub(super) fn compute_min_positive_priority(proposals: &[Proposal]) -> f64 {
    proposals
        .iter()
        .map(|p| p.priority)
        .filter(|&p| p > 0.0)
        .reduce(f64::min)
        .unwrap_or(1.0)
}

/// Compute the conflict group for placing `task` starting at `start`:
/// - All placed tasks overlapping `[start, start + duration)`.
/// - Closed over dependency descendants in the same block that are also placed.
pub(super) fn compute_conflict_group(
    start: Time<MJD>,
    task: &Task,
    schedule: &Schedule,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
) -> HashSet<TaskId> {
    let duration_days = task.duration.to::<Day>().value();
    let end = Time::<MJD>::new(start.value() + duration_days);
    let interval = Period::new(start, end);

    let initial: HashSet<TaskId> = schedule.overlapping(&interval).into_iter().collect();
    let mut closed = initial.clone();

    for &base_id in &initial {
        if let Some(&block_id) = task_to_block.get(&base_id)
            && let Some(block) = blocks.get(&block_id)
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

fn find_proposal_for_task<'a>(
    task_id: TaskId,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    proposals: &'a [Proposal],
) -> Option<&'a Proposal> {
    let &block_id = task_to_block.get(&task_id)?;
    proposals.iter().find(|p| p.id == block_id)
}

/// Compute the total conflict cost of placing `task` at `start`.
///
/// Cost = sum over conflicting tasks of:
/// - `conflicting_proposal.priority + impatience * (min_positive_priority / alpha)`
///   when the conflicting task belongs to a currently-complete proposal.
/// - `0.0` otherwise.
#[allow(clippy::too_many_arguments)]
pub(super) fn compute_conflict_cost(
    start: Time<MJD>,
    task: &Task,
    schedule: &Schedule,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    all_proposals: &[Proposal],
    current_proposal: &Proposal,
    min_positive_priority: f64,
    config: &Configuration,
) -> f64 {
    let impatience =
        current_proposal.task_count() as f64 / current_proposal.impatience_denominator as f64;
    let conflict_group = compute_conflict_group(start, task, schedule, task_to_block, blocks);

    let mut total_cost = 0.0;
    for conflicting_id in &conflict_group {
        let Some(proposal) = find_proposal_for_task(*conflicting_id, task_to_block, all_proposals)
        else {
            continue;
        };
        if proposal.is_complete(schedule) {
            total_cost +=
                proposal.priority + impatience * (min_positive_priority / config.impatience_alpha);
        }
    }
    total_cost
}
