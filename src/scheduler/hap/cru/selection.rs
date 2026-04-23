//! Candidate selection logic for CRU lobby processing.

use super::rng::Xorshift64;
use super::task_graph::predecessor_end_lower_bound;
use super::windows::generate_candidate_starts;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId, TaskId, Time};
use std::collections::HashMap;

/// Estimate the number of valid start positions for `task_id` given the
/// current schedule state. Used as a proxy for flexibility.
fn compute_flexibility(
    task_id: TaskId,
    tasks: &HashMap<TaskId, Task>,
    possible_periods: &TaskPeriodMap,
    schedule: &Schedule,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    horizon: &Period<MJD>,
) -> usize {
    let Some(task) = tasks.get(&task_id) else {
        return 0;
    };
    let Some(windows) = possible_periods.get(&task_id) else {
        return 0;
    };
    let pred_end = predecessor_end_lower_bound(task_id, schedule, blocks, task_to_block, horizon);
    generate_candidate_starts(task, windows, schedule, pred_end).len()
}

fn get_priority(task_id: TaskId, tasks: &HashMap<TaskId, Task>, at: Time<MJD>) -> f64 {
    tasks
        .get(&task_id)
        .and_then(|t| {
            t.soft_constraints
                .as_ref()
                .map(|sc| sc.score(&at, None, Some(&t.target)))
        })
        .unwrap_or(0.0)
}

/// Pick the best next candidate from `group`:
/// sort by `(flexibility ASC, priority DESC, task_id ASC)`.
pub(super) fn select_best_candidate(
    group: &[TaskId],
    tasks: &HashMap<TaskId, Task>,
    possible_periods: &TaskPeriodMap,
    schedule: &Schedule,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    horizon: &Period<MJD>,
) -> TaskId {
    *group
        .iter()
        .min_by(|&&a, &&b| {
            let flex_a = compute_flexibility(
                a,
                tasks,
                possible_periods,
                schedule,
                blocks,
                task_to_block,
                horizon,
            );
            let flex_b = compute_flexibility(
                b,
                tasks,
                possible_periods,
                schedule,
                blocks,
                task_to_block,
                horizon,
            );
            let flex_cmp = flex_a.cmp(&flex_b);
            if flex_cmp != std::cmp::Ordering::Equal {
                return flex_cmp;
            }
            let prio_a = get_priority(a, tasks, horizon.start);
            let prio_b = get_priority(b, tasks, horizon.start);
            let prio_cmp = prio_b.total_cmp(&prio_a); // DESC
            if prio_cmp != std::cmp::Ordering::Equal {
                return prio_cmp;
            }
            a.0.cmp(&b.0) // ASC tie-break
        })
        .expect("group must be non-empty")
}

/// Choose a candidate index from `costs`.
///
/// - If any cost is `0.0`: return the index of the first zero-cost candidate.
/// - Otherwise: sort by cost ascending and pick uniformly from the best
///   `stochastic_range` candidates using `rng`.
pub(super) fn choose_candidate(
    costs: &[f64],
    stochastic_range: usize,
    rng: &mut Xorshift64,
) -> usize {
    if costs.is_empty() {
        return 0;
    }
    let min_cost = costs.iter().cloned().fold(f64::INFINITY, f64::min);

    if min_cost == 0.0 {
        costs.iter().position(|&c| c == 0.0).unwrap_or(0)
    } else {
        let mut indexed: Vec<(usize, f64)> = costs.iter().copied().enumerate().collect();
        indexed.sort_by(|a, b| a.1.total_cmp(&b.1));
        let range = stochastic_range.min(indexed.len()).max(1);
        let chosen = rng.next_usize(range);
        indexed[chosen].0
    }
}
