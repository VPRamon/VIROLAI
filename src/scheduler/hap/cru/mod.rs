//! CRU (Conflict Resolution Unit) for the HAP scheduler.
//!
//! The CRU heuristic has two nested cycles:
//!
//! 1. The **Block Scheduling Cycle** ([`run_branches`]) iterates over every
//!    valid completion alternative (DNF branch) of the input block. Each
//!    branch starts from a fresh clone of the input schedule.
//! 2. The **Task Scheduling Cycle**
//!    ([`task_scheduler::task_scheduling_cycle`]) drains a lobby of displaced
//!    tasks for one block task while tracking the lowest-cost intermediate
//!    schedule (`s_low`) and restoring it on exit.
//!
//! [`run_branches`] returns every completed schedule produced by a satisfied
//! branch (deduplicated by canonical placement fingerprint).

pub mod lobby;
pub mod task_scheduler;

use super::configuration::Configuration;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::scheduling_block::SchedulingBlock;
use crate::time::TaskId;
use rand::Rng;
use std::collections::{HashMap, HashSet};

/// Run CRU over `block` for every valid completion branch, returning all
/// resulting completed schedules.
///
/// Each returned schedule starts from a fresh clone of `input` (so OR-branch
/// state never leaks across alternatives) and satisfies the block's
/// completion expression. When the block has no completion expression, the
/// implicit "every task scheduled" rule is used.
///
/// `all_blocks` resolves displaced tasks back to their owning block so the
/// inner cycle can keep using the correct protected set.
///
/// Schedules are deduplicated by canonical placement fingerprint
/// `(task_id, start, end)` so identical results from different branches are
/// emitted only once.
pub fn run_branches(
    input: &Schedule,
    block: &SchedulingBlock,
    all_blocks: &[SchedulingBlock],
    periods_map: &TaskPeriodMap,
    config: &Configuration,
    rng: &mut impl Rng,
) -> Vec<Schedule> {
    let task_index: HashMap<TaskId, (&crate::task::Task, &SchedulingBlock)> = all_blocks
        .iter()
        .flat_map(|b| b.iter_tasks().map(move |t| (t.id, (t, b))))
        .collect();

    let branches = match block.completion_branches() {
        Some(b) => b,
        None => return Vec::new(),
    };

    let mut out: Vec<Schedule> = Vec::new();
    let mut seen: HashSet<Vec<(u64, u64, u64)>> = HashSet::new();

    for branch in branches {
        let mut working = input.clone();

        for task_id in &branch {
            task_scheduler::task_scheduling_cycle(
                *task_id,
                &mut working,
                &task_index,
                periods_map,
                config,
                rng,
            );
        }

        if !block.is_complete(&working) {
            continue;
        }

        let fp = fingerprint(&working);
        if seen.insert(fp) {
            out.push(working);
        }
    }

    out
}

fn fingerprint(schedule: &Schedule) -> Vec<(u64, u64, u64)> {
    let mut v: Vec<(u64, u64, u64)> = schedule
        .placements()
        .map(|p| {
            (
                p.task_id.0,
                p.start.value().to_bits(),
                p.end.value().to_bits(),
            )
        })
        .collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::ConstraintBlocks;
    use crate::scheduling_block::CompletionExpr;
    use crate::task::Task;
    use crate::time::{MJD, Period, PeriodSet, SchedulingBlockId, Time};
    use qtty::Seconds;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use siderust::coordinates::frames::ICRS;
    use siderust::coordinates::spherical::Direction;

    fn task(id: u64, duration_days: f64) -> Task {
        Task::new(
            TaskId(id),
            format!("t{id}"),
            Direction::<ICRS>::new_raw(0.0.into(), 0.0.into()),
            Seconds::new(duration_days * 86400.0),
            ConstraintBlocks::default(),
            None,
        )
        .unwrap()
    }

    fn period(s: f64, e: f64) -> Period<MJD> {
        Period::new(Time::<MJD>::new(s), Time::<MJD>::new(e))
    }

    fn windows(s: f64, e: f64) -> PeriodSet<MJD> {
        PeriodSet::from_periods(vec![period(s, e)])
    }

    fn rng() -> StdRng {
        StdRng::seed_from_u64(0)
    }

    fn cfg() -> Configuration {
        Configuration::default()
    }

    /// `(t1 ∧ t2) ∨ t3` should produce two distinct completed schedules.
    #[test]
    fn or_branch_enumeration_returns_two_schedules() {
        let mut block = SchedulingBlock::from_tasks(
            SchedulingBlockId(1),
            vec![task(1, 1.0), task(2, 1.0), task(3, 1.0)],
        )
        .unwrap();
        block
            .set_completion(CompletionExpr::Or(vec![
                CompletionExpr::all_of([TaskId(1), TaskId(2)]),
                CompletionExpr::Leaf(TaskId(3)),
            ]))
            .unwrap();

        let mut periods_map = TaskPeriodMap::new();
        periods_map.insert(TaskId(1), windows(0.0, 5.0));
        periods_map.insert(TaskId(2), windows(0.0, 5.0));
        periods_map.insert(TaskId(3), windows(0.0, 5.0));

        let input = Schedule::new();
        let mut r = rng();
        let results = run_branches(
            &input,
            &block,
            &[block_clone(&block)],
            &periods_map,
            &cfg(),
            &mut r,
        );

        assert_eq!(results.len(), 2);
        // Branch 1: t1 + t2 placed; Branch 2: only t3 placed.
        let lens: Vec<usize> = results.iter().map(|s| s.len()).collect();
        assert!(lens.contains(&2));
        assert!(lens.contains(&1));

        let only_t3 = results.iter().find(|s| s.len() == 1).unwrap();
        assert!(only_t3.contains(TaskId(3)));
        assert!(!only_t3.contains(TaskId(1)));
        assert!(!only_t3.contains(TaskId(2)));
    }

    /// Branch isolation: the `t3` branch must NOT carry placements of
    /// `t1` / `t2` from the other branch.
    #[test]
    fn branches_do_not_leak_state() {
        let mut block = SchedulingBlock::from_tasks(
            SchedulingBlockId(1),
            vec![task(1, 1.0), task(2, 1.0), task(3, 1.0)],
        )
        .unwrap();
        block
            .set_completion(CompletionExpr::Or(vec![
                CompletionExpr::all_of([TaskId(1), TaskId(2)]),
                CompletionExpr::Leaf(TaskId(3)),
            ]))
            .unwrap();

        let mut periods_map = TaskPeriodMap::new();
        periods_map.insert(TaskId(1), windows(0.0, 5.0));
        periods_map.insert(TaskId(2), windows(0.0, 5.0));
        periods_map.insert(TaskId(3), windows(0.0, 5.0));

        let input = Schedule::new();
        let mut r = rng();
        let results = run_branches(
            &input,
            &block,
            &[block_clone(&block)],
            &periods_map,
            &cfg(),
            &mut r,
        );

        for s in &results {
            let has_t1_or_t2 = s.contains(TaskId(1)) || s.contains(TaskId(2));
            let has_t3 = s.contains(TaskId(3));
            assert!(!(has_t1_or_t2 && has_t3), "branch leaked state");
        }
    }

    /// Identical schedules from two branches should collapse to one.
    #[test]
    fn dedupes_equivalent_schedules() {
        let mut block =
            SchedulingBlock::from_tasks(SchedulingBlockId(1), vec![task(1, 1.0)]).unwrap();
        block
            .set_completion(CompletionExpr::Or(vec![
                CompletionExpr::Leaf(TaskId(1)),
                CompletionExpr::Leaf(TaskId(1)),
            ]))
            .unwrap();

        let mut periods_map = TaskPeriodMap::new();
        periods_map.insert(TaskId(1), windows(0.0, 5.0));

        let input = Schedule::new();
        let mut r = rng();
        let results = run_branches(
            &input,
            &block,
            &[block_clone(&block)],
            &periods_map,
            &cfg(),
            &mut r,
        );
        assert_eq!(results.len(), 1);
    }

    /// When no branch can be completed, the result set is empty.
    #[test]
    fn returns_empty_when_no_branch_completes() {
        let mut block =
            SchedulingBlock::from_tasks(SchedulingBlockId(1), vec![task(1, 1.0)]).unwrap();
        block
            .set_completion(CompletionExpr::Leaf(TaskId(1)))
            .unwrap();

        // No periods -> task can't be scheduled -> branch incomplete.
        let periods_map = TaskPeriodMap::new();

        let input = Schedule::new();
        let mut r = rng();
        let results = run_branches(
            &input,
            &block,
            &[block_clone(&block)],
            &periods_map,
            &cfg(),
            &mut r,
        );
        assert!(results.is_empty());
    }

    /// Two runs with the deterministic selector and identical inputs must
    /// produce identical placement fingerprints.
    #[test]
    fn deterministic_run_branches_is_reproducible() {
        let mut block =
            SchedulingBlock::from_tasks(SchedulingBlockId(1), vec![task(1, 1.0), task(2, 1.0)])
                .unwrap();
        block
            .set_completion(CompletionExpr::all_of([TaskId(1), TaskId(2)]))
            .unwrap();

        let mut periods_map = TaskPeriodMap::new();
        periods_map.insert(TaskId(1), windows(0.0, 5.0));
        periods_map.insert(TaskId(2), windows(0.0, 5.0));

        let go = || {
            let mut r = rng();
            run_branches(
                &Schedule::new(),
                &block,
                &[block_clone(&block)],
                &periods_map,
                &cfg(),
                &mut r,
            )
            .into_iter()
            .map(|s| fingerprint(&s))
            .collect::<Vec<_>>()
        };
        assert_eq!(go(), go());
    }

    /// Test helper: rebuild a block by re-creating its tasks with the same IDs.
    /// Real code never needs to clone blocks, but tests pass `&[SchedulingBlock]`
    /// and we want the same block in `all_blocks` as the one we're running.
    fn block_clone(b: &SchedulingBlock) -> SchedulingBlock {
        let tasks: Vec<Task> = b.iter_tasks().map(|t| task(t.id.0, 1.0)).collect();
        let mut nb = SchedulingBlock::from_tasks(b.id, tasks).unwrap();
        if let Some(expr) = b.completion() {
            nb.set_completion(expr.clone()).unwrap();
        }
        nb
    }
}
