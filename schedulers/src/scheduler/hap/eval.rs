//! Schedule and block evaluators shared by AP and HAP.
//!
//! These helpers are deliberately kept free of any planner-specific state so
//! the same metrics can be computed from a `Schedule` plus the global
//! [`SchedulingProblem`] context.
//!
//! Lifted from the archived HAP implementation
//! (`src/scheduler/archive/hap/{block_eval,ranking}.rs`) so the new AP/HAP
//! orchestration can reuse the same definitions.

use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduling_block::SchedulingBlock;
use crate::time::{MJD, Time};

/// Sum of soft-constraint scores of every task in `block`, evaluated at
/// `horizon_start`. Used as the block-level priority.
pub fn block_priority(
    block: &SchedulingBlock,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> f64 {
    block
        .iter()
        .map(|id| {
            problem
                .task(id)
                .and_then(|task| {
                    task.soft_constraints.as_ref().map(|constraints| {
                        constraints.score(&horizon_start, None, Some(&task.target))
                    })
                })
                .unwrap_or(0.0)
        })
        .sum()
}

/// Number of placed tasks divided by total task count across all blocks
/// in `problem`. Returns `0.0` when the problem has no tasks.
pub fn scheduling_rate(schedule: &Schedule, problem: &SchedulingProblem) -> f64 {
    let total = problem.task_count();
    if total == 0 {
        return 0.0;
    }
    schedule.len() as f64 / total as f64
}

/// Sum of `block_priority` over every block whose completion expression
/// is satisfied by `schedule`.
pub fn priority_sum(
    schedule: &Schedule,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> f64 {
    problem
        .blocks()
        .iter()
        .filter(|b| b.is_complete(schedule))
        .map(|b| block_priority(b, problem, horizon_start))
        .sum()
}

/// Priority-weighted completion fitness: `Σ_b (placed_b / total_b) ·
/// block_priority(b)`. Higher is better. Matches the archived HAP scalar.
pub fn completion_fitness(
    schedule: &Schedule,
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> f64 {
    problem
        .blocks()
        .iter()
        .map(|block| {
            let total = block.iter().count();
            if total == 0 {
                return 0.0;
            }
            let placed = block.iter().filter(|id| schedule.contains(*id)).count();
            (placed as f64 / total as f64) * block_priority(block, problem, horizon_start)
        })
        .sum()
}

/// Sum of placement durations (end - start) — proxy for total science time.
pub fn science_time(schedule: &Schedule) -> f64 {
    schedule
        .placements()
        .map(|p| p.end.value() - p.start.value())
        .sum()
}

/// Canonical fingerprint of a schedule: sorted `(task_id, start_bits,
/// end_bits)` tuples. Two schedules with the same fingerprint contain the
/// same placements and are interchangeable for planner purposes.
pub fn placement_fingerprint(schedule: &Schedule) -> Vec<(u64, u64, u64)> {
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
    v.sort_unstable();
    v
}
