//! CRU (Conflict Resolution Unit) for the HAP scheduler.
//!
//! The [`run`] function drives the outer Block Scheduling Cycle described in
//! the CRU algorithm: for every task in a block it initialises a [`Lobby`]
//! with that task and drains it via [`task_scheduler::schedule_task`] (the
//! Task Scheduling Cycle inner step) until the lobby is empty or the
//! iteration budget [`Configuration::max_iter`] is exhausted.
//!
//! Task dependencies within the block are **not** considered in this
//! implementation; tasks are processed in their input order.

pub mod lobby;
pub mod task_scheduler;

use self::lobby::Lobby;
use super::configuration::Configuration;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::scheduling_block::SchedulingBlock;
use crate::time::TaskId;
use rand::Rng;
use std::collections::{HashMap, HashSet};

/// Place all tasks from `block` into `schedule` using the CRU algorithm.
///
/// `all_blocks` is the full set of blocks in the problem.  When a displaced
/// task belongs to a block other than the one currently being added, it is
/// looked up from `all_blocks` so it can be rescheduled with its own block
/// as the protected set.
///
/// For each task in `block` the algorithm:
/// 1. Seeds the lobby with that task.
/// 2. Repeatedly pops a task from the lobby and calls
///    [`task_scheduler::schedule_task`], which may evict non-protected tasks
///    back into the lobby.
/// 3. Stops the inner loop when the lobby is empty or `config.max_iter`
///    iterations have been used.
///
/// Tasks whose `TaskId` cannot be resolved across all blocks (should not
/// happen in a well-formed problem) are silently dropped.  Task scheduling
/// errors (no feasibility windows, no valid candidates) are also ignored.
pub fn run(
    schedule: &mut Schedule,
    block: &SchedulingBlock,
    all_blocks: &[SchedulingBlock],
    periods_map: &TaskPeriodMap,
    config: &Configuration,
    rng: &mut impl Rng,
) {
    // Build a flat TaskId -> (&Task, &SchedulingBlock) index once per call.
    let task_index: HashMap<TaskId, (&crate::task::Task, &SchedulingBlock)> = all_blocks
        .iter()
        .flat_map(|b| b.iter_tasks().map(move |t| (t.id, (t, b))))
        .collect();

    for task in block.iter_tasks() {
        let mut lobby = Lobby::new();
        lobby.push(task.id);

        let mut run_protected: HashSet<TaskId> = HashSet::new();
        let mut iter = 0usize;
        while let Some(task_id) = lobby.pop() {
            if iter >= config.max_iter {
                break;
            }
            iter += 1;

            let (t, owner_block) = task_index
                .get(&task_id)
                .expect("task_id not found in any block — problem is not well-formed");

            if task_scheduler::schedule_task(
                t,
                owner_block,
                schedule,
                periods_map,
                &mut lobby,
                &run_protected,
                config,
                rng,
            )
            .is_ok()
            {
                run_protected.insert(task_id);
            }
        }
    }
}
