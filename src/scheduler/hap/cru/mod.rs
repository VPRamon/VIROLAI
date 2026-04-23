//! CRU (Conflict Resolution Unit) engine for the HAP scheduler.
//!
//! Each CRU starts from a survivor schedule and attempts to place all tasks of
//! one proposal (SchedulingBlock) by evicting conflicting non-protected
//! tasks when necessary. The best intermediate state is tracked and returned
//! when the iteration budget is exhausted or all protected tasks are placed.

mod conflict;
mod rng;
mod selection;
mod snapshot;
mod task_graph;
mod windows;

#[cfg(test)]
mod tests;

use self::conflict::{
    compute_conflict_cost, compute_conflict_group, compute_min_positive_priority,
};
use self::rng::Xorshift64;
use self::selection::{choose_candidate, select_best_candidate};
use self::snapshot::{count_protected_placed, count_unplaced_displaced, is_better_snapshot};
use self::task_graph::{predecessor_end_lower_bound, predecessors_placed};
use self::windows::{generate_candidate_starts, would_evict_protected};
use super::configuration::Configuration;
use super::proposal::Proposal;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, TaskPlacement};
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId, TaskId, Time};
use qtty::Day;
use std::collections::{HashMap, HashSet};

/// Run one CRU repair pass starting from `base_schedule`.
///
/// Attempts to place all tasks in `proposal` (the *protected set*).
/// Non-protected conflicting tasks may be evicted and re-queued in the
/// internal displaced lobby. Returns the best intermediate schedule
/// observed during the run.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_cru(
    base_schedule: Schedule,
    proposal: &Proposal,
    tasks: &HashMap<TaskId, Task>,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    task_to_block: &HashMap<TaskId, SchedulingBlockId>,
    all_proposals: &[Proposal],
    config: &Configuration,
    seed: u64,
) -> Schedule {
    let mut rng = Xorshift64::new(seed);
    let protected_ids: HashSet<TaskId> = proposal.task_ids.iter().copied().collect();
    let mut displaced: HashSet<TaskId> = HashSet::new();
    let mut schedule = base_schedule;

    let mut best_protected_placed = count_protected_placed(&schedule, &protected_ids);
    let mut best_unplaced_displaced = 0usize;
    let mut best_snapshot = schedule.clone();

    let min_positive_priority = compute_min_positive_priority(all_proposals);

    for iteration in 0..config.cru_max_iterations {
        // Build lobby: unplaced protected tasks ∪ unplaced displaced tasks
        let lobby: HashSet<TaskId> = protected_ids
            .iter()
            .filter(|id| !schedule.contains(**id))
            .chain(displaced.iter().filter(|id| !schedule.contains(**id)))
            .copied()
            .collect();

        if lobby.is_empty() {
            break;
        }

        // Find ready tasks: all predecessors in their block are placed
        let ready: Vec<TaskId> = lobby
            .iter()
            .copied()
            .filter(|&tid| predecessors_placed(tid, &schedule, blocks, task_to_block))
            .collect();

        if ready.is_empty() {
            break;
        }

        // Prefer protected tasks; fall back to displaced when none are ready
        let ready_protected: Vec<TaskId> = ready
            .iter()
            .copied()
            .filter(|id| protected_ids.contains(id))
            .collect();
        let ready_displaced: Vec<TaskId> = ready
            .iter()
            .copied()
            .filter(|id| !protected_ids.contains(id))
            .collect();

        let candidate_group = if !ready_protected.is_empty() {
            ready_protected
        } else {
            ready_displaced
        };

        let next_task_id = select_best_candidate(
            &candidate_group,
            tasks,
            possible_periods,
            &schedule,
            blocks,
            task_to_block,
            horizon,
        );

        let Some(task) = tasks.get(&next_task_id) else {
            log::warn!("hap cru: task {} not found in task map", next_task_id.0);
            break;
        };
        let Some(windows) = possible_periods.get(&next_task_id) else {
            log::debug!("hap cru: task {} has no feasible windows", next_task_id.0);
            break;
        };

        // Predecessor lower bound on start time
        let pred_end =
            predecessor_end_lower_bound(next_task_id, &schedule, blocks, task_to_block, horizon);

        // Generate and filter candidate starts
        let candidate_starts = generate_candidate_starts(task, windows, &schedule, pred_end);
        let candidate_starts: Vec<Time<MJD>> = candidate_starts
            .into_iter()
            .filter(|&s| !would_evict_protected(s, task, &schedule, &protected_ids))
            .collect();

        if candidate_starts.is_empty() {
            log::debug!(
                "hap cru: no valid windows for task {} after protected filter",
                next_task_id.0
            );
            break;
        }

        // Compute conflict cost for every candidate start
        let costs: Vec<f64> = candidate_starts
            .iter()
            .map(|&s| {
                compute_conflict_cost(
                    s,
                    task,
                    &schedule,
                    task_to_block,
                    blocks,
                    all_proposals,
                    proposal,
                    min_positive_priority,
                    config,
                )
            })
            .collect();

        // Choose the candidate window
        let chosen_idx = choose_candidate(&costs, config.stochastic_range, &mut rng);
        let chosen_start = candidate_starts[chosen_idx];

        // Compute conflict group for the chosen window and evict non-protected
        let conflict_group =
            compute_conflict_group(chosen_start, task, &schedule, task_to_block, blocks);
        for &evicted_id in &conflict_group {
            if !protected_ids.contains(&evicted_id) {
                let _ = schedule.unplace_task(evicted_id);
                displaced.insert(evicted_id);
            }
        }

        // Place the task
        let end = Time::<MJD>::new(chosen_start.value() + task.duration.to::<Day>().value());
        let block_id = task_to_block.get(&next_task_id).copied();
        schedule.insert_placement(TaskPlacement {
            task_id: next_task_id,
            start: chosen_start,
            end,
            block_id,
        });
        displaced.remove(&next_task_id);

        log::trace!(
            "hap cru: iter={} placed task={} at [{:.4}, {:.4}]",
            iteration,
            next_task_id.0,
            chosen_start.value(),
            end.value(),
        );

        // Update best snapshot
        let new_protected_placed = count_protected_placed(&schedule, &protected_ids);
        let new_unplaced_displaced = count_unplaced_displaced(&schedule, &displaced);
        if is_better_snapshot(
            new_protected_placed,
            new_unplaced_displaced,
            &schedule,
            best_protected_placed,
            best_unplaced_displaced,
            &best_snapshot,
            all_proposals,
        ) {
            best_snapshot = schedule.clone();
            best_protected_placed = new_protected_placed;
            best_unplaced_displaced = new_unplaced_displaced;
        }

        // Early exit when all protected tasks are placed
        if new_protected_placed == protected_ids.len() {
            log::debug!(
                "hap cru: all {} protected task(s) placed, returning early",
                protected_ids.len()
            );
            return schedule;
        }
    }

    best_snapshot
}
