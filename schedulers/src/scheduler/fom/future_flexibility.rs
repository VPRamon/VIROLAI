//! FOM: feasibility-first future flexibility for EST beam search.
//!
//! This module implements [`FutureFlexibilityFom`], which scores partial
//! schedule states by how well they preserve the ability to schedule remaining
//! tasks, rather than purely by the quality already captured.
//!
//! ## Direction and multi-cursor awareness
//!
//! When the cursor engine supplies `ctx.active_periods`, each non-`None` entry
//! is the post-placement residual region of the corresponding cursor. The FOM
//! evaluates each unplaced task in every active region, then assigns that task
//! to the region where it retains the **maximum** residual flexibility:
//!
//! * Single-forward (EST): one region `[placement.end, horizon.end]` —
//!   identical to the legacy single-frontier behaviour.
//! * Single-backward (LST): one region `[horizon.start, placement.start]` —
//!   correctly covers the backward-filling zone.
//! * Multi-cursor (e.g. `dynamic_est_lst_meet`): several regions, one per
//!   cursor. A task is recoverable if **any** region can still schedule it, and
//!   density is normalized by the total active-region duration.
//!
//! ## Fallback
//!
//! When `active_periods` is absent (unit tests, legacy call sites) the FOM
//! falls back to a single synthetic region `[ctx.cursor, ctx.horizon.end]`,
//! preserving historic single-frontier behaviour exactly.
//!
//! ## References
//!
//! - Policella et al. 2004 — robustness as retained temporal flexibility.
//! - Smith & Cheng 1993 — cheap slack-based heuristics guide search well.
//! - Kramer & Smith 2006 — simple containment metrics often yield better
//!   cost/quality in oversubscribed problems.
//! - Giuliano et al. 2007 — plan windows preserve global flexibility in astronomy scheduling.

use super::{FomContext, ScheduleFom};
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::task::Task;
use crate::time::{MJD, Period, TaskId, Time};
use qtty::Day;

/// FOM: feasibility-first future flexibility.
///
/// Scores a partial schedule state by how well it preserves the ability to
/// schedule remaining tasks. The signal decomposes into four sub-signals ranked
/// with stable lexicographic weights:
///
/// ```text
/// score = 10 * recoverable_count
///          +  density_term
///          + 0.1 * reserve_term
///          + 0.01 * soft_term
/// ```
///
/// | Term                 | Meaning |
/// |----------------------|---------|
/// | `recoverable_count`  | Placed tasks + unplaced tasks with ≥ 1 full placement window from cursor. |
/// | `density_term`       | `1 / (1 + overload_area / remaining_horizon)` — penalises crowded futures. |
/// | `reserve_term`       | Mean `1 − 1/flexibility` over recoverable unplaced tasks — rewards slack. |
/// | `soft_term`          | Soft-constraint quality normalised to `[0, 1]` — small already-captured signal. |
///
/// ## Dependency handling
///
/// A task is not counted as recoverable if any predecessor in its scheduling
/// block is not yet scheduled. If all predecessors are placed but one ends
/// *after* the current beam cursor, the effective cursor for that task is
/// shifted to `max(cursor, max_pred_end)`, which reduces the available
/// flexibility accordingly.
#[derive(Debug, Clone, Default)]
pub struct FutureFlexibilityFom;

impl ScheduleFom for FutureFlexibilityFom {
    /// Score the partial schedule for its future-flexibility potential.
    fn evaluate(
        &self,
        schedule: &Schedule,
        problem: &SchedulingProblem,
        ctx: &FomContext<'_>,
    ) -> f64 {
        let Some(possible_periods) = ctx.possible_periods else {
            // No feasibility windows available; degrade gracefully to placed count.
            return schedule.len() as f64;
        };

        // Collect all active regions supplied by the cursor engine.
        //
        // `ctx.active_periods` is index-parallel to the cursor list; each
        // non-`None` entry is that cursor's post-placement residual region.
        // We collect all non-None entries to evaluate recoverability over the
        // *union* of active regions (any cursor can place any remaining task).
        //
        // When `active_periods` is absent (unit tests, legacy paths), fall back
        // to the legacy `[cursor, horizon.end]` single-frontier region.
        let active_regions: Vec<Period<MJD>> = match ctx.active_periods {
            Some(aps) => {
                let v: Vec<Period<MJD>> = aps.iter().flatten().copied().collect();
                if v.is_empty() {
                    vec![Period::new(ctx.cursor, ctx.horizon.end)]
                } else {
                    v
                }
            }
            None => vec![Period::new(ctx.cursor, ctx.horizon.end)],
        };

        let placed_count = schedule.len() as f64;
        let task_count = problem.task_count();

        // Classify each unplaced task and compute its best residual flexibility
        // across all active cursor regions.
        //
        // Each entry is (task_id, best_flex, eff_cursor, region_end) where
        // `eff_cursor` and `region_end` come from the active region in which
        // the task achieves the highest flexibility.
        let mut residual_tasks: Vec<(TaskId, f64, Time<MJD>, Time<MJD>)> = Vec::new();

        for task in problem.iter_tasks() {
            if schedule.get(task.id).is_some() {
                continue; // already placed
            }

            let windows = match possible_periods.get(&task.id) {
                Some(w) => w,
                None => continue,
            };

            let mut best_flex = 0.0_f64;
            let mut best_eff_cursor = active_regions[0].start;
            let mut best_region_end = active_regions[0].end;

            for active in &active_regions {
                // Determine effective cursor accounting for placed predecessors.
                // `active.start` is the baseline for this cursor's region:
                //   forward  → `placement.end`
                //   backward → `horizon.start`
                let Some(eff_cursor) =
                    effective_cursor_for(task.id, active.start, schedule, problem)
                else {
                    // At least one predecessor is unscheduled → task is blocked
                    // regardless of which cursor we consider.
                    continue;
                };

                let eff_horizon = Period::new(eff_cursor, active.end);
                let flex = residual_flexibility_for(task, windows.as_slice(), eff_horizon);

                if flex > best_flex {
                    best_flex = flex;
                    best_eff_cursor = eff_cursor;
                    best_region_end = active.end;
                }
            }

            if best_flex >= 1.0 {
                residual_tasks.push((task.id, best_flex, best_eff_cursor, best_region_end));
            }
        }

        let recoverable_count = placed_count + residual_tasks.len() as f64;
        let density_term = compute_density_term(&residual_tasks, possible_periods, &active_regions);
        let reserve_term = compute_reserve_term(&residual_tasks);
        let soft_term = compute_soft_term(schedule, problem, task_count);

        10.0 * recoverable_count + density_term + 0.1 * reserve_term + 0.01 * soft_term
    }

    fn label(&self) -> &'static str {
        "future_flexibility"
    }
}

/// Compute the effective cursor for a task, accounting for scheduled predecessors.
///
/// Returns `None` if any predecessor in the same block is not yet scheduled
/// (the task is then considered unrecoverable in the current state).
///
/// Returns `Some(t)` where `t = max(cursor, max_pred_end)` once all
/// predecessors are confirmed placed.
fn effective_cursor_for(
    task_id: TaskId,
    cursor: Time<MJD>,
    schedule: &Schedule,
    problem: &SchedulingProblem,
) -> Option<Time<MJD>> {
    let block_id = match problem.task_block_id(task_id) {
        Some(id) => id,
        None => return Some(cursor), // no block context — no dependency constraints
    };
    let block = match problem.block(block_id) {
        Some(b) => b,
        None => return Some(cursor),
    };

    let predecessors = block.all_predecessors(task_id);
    let mut max_pred_end = cursor;

    for pred_id in predecessors {
        match schedule.get(pred_id) {
            None => return None, // predecessor not scheduled → task is blocked
            Some(placement) => {
                if placement.end > max_pred_end {
                    max_pred_end = placement.end;
                }
            }
        }
    }

    Some(max_pred_end)
}

/// Compute the residual flexibility of `task` over `effective_horizon`.
///
/// Replicates the window-accumulation logic of `Candidate::refresh` but
/// operates on a raw window slice without mutating any candidate state.
///
/// Flexibility is expressed as the number of task-length units of feasible
/// time remaining across all valid window overlaps.
pub(crate) fn residual_flexibility_for(
    task: &Task,
    windows: &[Period<MJD>],
    effective_horizon: Period<MJD>,
) -> f64 {
    let duration_days = task.duration.to::<Day>().value();
    let mut flexibility = 0.0;

    for window in windows {
        if window.end <= effective_horizon.start {
            continue;
        }
        if window.start >= effective_horizon.end {
            break;
        }
        let Some(overlap) = window.intersection(&effective_horizon) else {
            continue;
        };
        let overlap_days = overlap.duration().to::<Day>().value();
        if overlap_days >= duration_days {
            flexibility += overlap_days / duration_days;
        }
    }

    flexibility
}

/// Build an event-sweep load profile and return the density term.
///
/// Each recoverable unplaced task with flexibility `F` contributes uniform
/// load `1/F` across all of its usable windows within its assigned active
/// region.  The overload area is the time-integral of `max(load − 1, 0)`.
///
/// `density_term = 1 / (1 + overload_area / remaining_horizon)`
///
/// `remaining_horizon` is the **sum** of all active-region durations so
/// that multi-cursor layouts are normalised correctly.
fn compute_density_term(
    residual_tasks: &[(TaskId, f64, Time<MJD>, Time<MJD>)],
    possible_periods: &TaskPeriodMap,
    active_regions: &[Period<MJD>],
) -> f64 {
    let remaining_horizon: f64 = active_regions
        .iter()
        .map(|r| (r.end.value() - r.start.value()).max(0.0))
        .sum();
    if remaining_horizon == 0.0 || residual_tasks.is_empty() {
        return 1.0;
    }

    // Build (time_value, load_delta) events for a temporal endpoint sweep.
    let mut events: Vec<(f64, f64)> = Vec::new();

    for &(task_id, flexibility, eff_cursor, region_end) in residual_tasks {
        let density = 1.0 / flexibility;
        let windows = match possible_periods.get(&task_id) {
            Some(w) => w,
            None => continue,
        };
        let eff_horizon = Period::new(eff_cursor, region_end);

        for window in windows.as_slice() {
            if window.end <= eff_cursor {
                continue;
            }
            if window.start >= region_end {
                break;
            }
            let Some(overlap) = window.intersection(&eff_horizon) else {
                continue;
            };
            events.push((overlap.start.value(), density));
            events.push((overlap.end.value(), -density));
        }
    }

    if events.is_empty() {
        return 1.0;
    }

    // Sort: earlier times first; at the same time, removals (−delta) before
    // additions (+delta) to avoid transient over-counting.
    events.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));

    let min_active_start = active_regions
        .iter()
        .map(|r| r.start.value())
        .fold(f64::INFINITY, f64::min);

    let mut load = 0.0_f64;
    let mut prev_t = events[0].0.max(min_active_start);
    let mut overload_area = 0.0_f64;

    for (t, delta) in &events {
        let t = t.max(min_active_start);
        let dt = t - prev_t;
        if dt > 0.0 && load > 1.0 {
            overload_area += (load - 1.0) * dt;
        }
        load += delta;
        prev_t = t;
    }

    1.0 / (1.0 + overload_area / remaining_horizon)
}

/// Compute the mean residual slack over recoverable unplaced tasks.
///
/// Each task contributes `1 − 1/flexibility`: zero when the task barely fits
/// (flexibility ≈ 1), approaching 1 when many placements remain.
///
/// Returns 0.0 when there are no recoverable unplaced tasks.
fn compute_reserve_term(residual_tasks: &[(TaskId, f64, Time<MJD>, Time<MJD>)]) -> f64 {
    if residual_tasks.is_empty() {
        return 0.0;
    }
    let sum: f64 = residual_tasks
        .iter()
        .map(|&(_, flex, _, _)| 1.0 - 1.0 / flex)
        .sum();
    sum / residual_tasks.len() as f64
}

/// Normalise the current soft-constraint quality to `[0, 1]`.
///
/// Uses the total task count as the normalisation reference so the scale
/// remains consistent across problem sizes.
fn compute_soft_term(schedule: &Schedule, problem: &SchedulingProblem, task_count: usize) -> f64 {
    if task_count == 0 {
        return 0.0;
    }
    let soft_raw: f64 = schedule
        .placements()
        .map(|p| {
            problem
                .task(p.task_id)
                .and_then(|task| {
                    task.soft_constraints
                        .as_ref()
                        .map(|sc| sc.score(&p.start.to::<MJD>(), None, Some(&task.target)))
                })
                .unwrap_or(0.0)
        })
        .sum();
    (soft_raw / task_count as f64).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::{ConstraintExpr, PrioritySoftConstraint, SoftConstraintExpr};
    use crate::prescheduler::TaskPeriodMap;
    use crate::schedule::{Schedule, SchedulingProblem, TaskPlacement};
    use crate::scheduling_block::{Dependency, SchedulingBlock};
    use crate::task::{IcrsTarget, Task};
    use crate::time::{MJD, Period, PeriodSet, SchedulingBlockId, TaskId, Time};
    use qtty::{Degrees, Seconds};
    use siderust::coordinates::frames::ICRS;
    use siderust::coordinates::spherical::Direction;
    use std::collections::HashMap;

    fn t(v: f64) -> Time<MJD> {
        Time::<MJD>::new(v)
    }

    fn period(s: f64, e: f64) -> Period<MJD> {
        Period::new(t(s), t(e))
    }

    fn target() -> IcrsTarget {
        Direction::<ICRS>::new_raw(Degrees::new(10.0), Degrees::new(20.0))
    }

    fn make_task(id: u64, duration_days: f64) -> Task {
        Task::new(
            TaskId(id),
            format!("task-{id}"),
            target(),
            Seconds::new(duration_days * 86_400.0),
            ConstraintExpr::Intersection(vec![]),
            None,
        )
        .expect("task should be valid")
    }

    fn make_task_with_priority(id: u64, duration_days: f64, priority: f64) -> Task {
        let soft = Some(SoftConstraintExpr::atom(PrioritySoftConstraint::new(
            priority,
        )));
        Task::new(
            TaskId(id),
            format!("task-{id}"),
            target(),
            Seconds::new(duration_days * 86_400.0),
            ConstraintExpr::Intersection(vec![]),
            soft,
        )
        .expect("task should be valid")
    }

    fn windows_set(periods: &[(f64, f64)]) -> PeriodSet<MJD> {
        PeriodSet::from_periods(periods.iter().map(|(s, e)| period(*s, *e)).collect())
    }

    fn place(schedule: &mut Schedule, id: u64, start: f64, end: f64) {
        schedule.insert_placement(TaskPlacement {
            task_id: TaskId(id),
            start: t(start),
            end: t(end),
        });
    }

    fn ctx_with<'a>(
        cursor: f64,
        horizon_s: f64,
        horizon_e: f64,
        periods: &'a TaskPeriodMap,
    ) -> FomContext<'a> {
        FomContext::single_cursor(t(cursor), period(horizon_s, horizon_e), Some(periods))
    }

    // ── residual_flexibility_for ─────────────────────────────────────────────

    #[test]
    fn impossible_task_has_zero_flexibility() {
        let task = make_task(1, 1.0);
        let windows = vec![period(0.0, 0.5)];
        let flex = residual_flexibility_for(&task, &windows, period(0.0, 1.0));
        assert_eq!(flex, 0.0, "window shorter than duration must yield 0");
    }

    #[test]
    fn single_exact_window_has_flexibility_one() {
        let task = make_task(1, 1.0);
        let windows = vec![period(0.0, 1.0)];
        let flex = residual_flexibility_for(&task, &windows, period(0.0, 2.0));
        assert!(
            (flex - 1.0).abs() < 1e-9,
            "exact fit → flexibility=1.0, got {flex}"
        );
    }

    #[test]
    fn more_windows_increase_flexibility() {
        let task = make_task(1, 1.0);
        let narrow = vec![period(0.0, 1.0)];
        let wide = vec![period(0.0, 1.0), period(2.0, 3.5)];
        let flex_narrow = residual_flexibility_for(&task, &narrow, period(0.0, 4.0));
        let flex_wide = residual_flexibility_for(&task, &wide, period(0.0, 4.0));
        assert!(
            flex_wide > flex_narrow,
            "more/wider windows must increase flexibility"
        );
    }

    #[test]
    fn windows_before_cursor_are_ignored() {
        let task = make_task(1, 1.0);
        let windows = vec![period(0.0, 2.0)];
        let flex = residual_flexibility_for(&task, &windows, period(1.5, 3.0));
        assert_eq!(
            flex, 0.0,
            "sub-duration overlap after cursor must be ignored"
        );
    }

    // ── FutureFlexibilityFom::evaluate ───────────────────────────────────────

    #[test]
    fn impossible_task_not_counted_as_recoverable() {
        let fom = FutureFlexibilityFom;
        let task = make_task(10, 1.0);
        let problem = SchedulingProblem::from_blocks(vec![
            SchedulingBlock::from_tasks(SchedulingBlockId(10), vec![task]).unwrap(),
        ])
        .unwrap();

        let mut periods: TaskPeriodMap = HashMap::new();
        periods.insert(TaskId(10), windows_set(&[(0.0, 0.5)]));

        let schedule = Schedule::new();
        let ctx = ctx_with(0.0, 0.0, 2.0, &periods);
        let score = fom.evaluate(&schedule, &problem, &ctx);
        assert!(
            score < 10.0,
            "impossible task must not inflate recoverable_count; score={score}"
        );
    }

    #[test]
    fn placed_tasks_always_count_toward_recoverable() {
        let fom = FutureFlexibilityFom;
        let task = make_task(20, 1.0);
        let problem = SchedulingProblem::from_blocks(vec![
            SchedulingBlock::from_tasks(SchedulingBlockId(20), vec![task]).unwrap(),
        ])
        .unwrap();

        let periods: TaskPeriodMap = HashMap::new();
        let mut schedule = Schedule::new();
        place(&mut schedule, 20, 0.0, 1.0);

        let ctx = ctx_with(1.0, 0.0, 5.0, &periods);
        let score = fom.evaluate(&schedule, &problem, &ctx);
        assert!(
            score >= 10.0,
            "placed task must count toward recoverable_count; score={score}"
        );
    }

    #[test]
    fn flexible_future_scores_higher_than_dense_future() {
        let fom = FutureFlexibilityFom;

        let task_a = make_task(30, 1.0);
        let prob_a = SchedulingProblem::from_blocks(vec![
            SchedulingBlock::from_tasks(SchedulingBlockId(30), vec![task_a]).unwrap(),
        ])
        .unwrap();
        let mut periods_a: TaskPeriodMap = HashMap::new();
        periods_a.insert(TaskId(30), windows_set(&[(0.0, 3.0)]));

        let task_b = make_task(31, 1.0);
        let prob_b = SchedulingProblem::from_blocks(vec![
            SchedulingBlock::from_tasks(SchedulingBlockId(31), vec![task_b]).unwrap(),
        ])
        .unwrap();
        let mut periods_b: TaskPeriodMap = HashMap::new();
        periods_b.insert(TaskId(31), windows_set(&[(0.0, 1.0)]));

        let schedule = Schedule::new();
        let score_a = fom.evaluate(&schedule, &prob_a, &ctx_with(0.0, 0.0, 4.0, &periods_a));
        let score_b = fom.evaluate(&schedule, &prob_b, &ctx_with(0.0, 0.0, 4.0, &periods_b));

        assert!(
            score_a > score_b,
            "flexible future (flex=3) must score higher than dense (flex=1); \
             score_a={score_a}, score_b={score_b}"
        );
    }

    #[test]
    fn blocked_predecessor_makes_task_not_recoverable() {
        let fom = FutureFlexibilityFom;

        let pred = make_task(40, 1.0);
        let succ = make_task(41, 1.0);

        let mut block = SchedulingBlock::new(SchedulingBlockId(40));
        block.push_task(pred).unwrap();
        block.push_task(succ).unwrap();
        block
            .add_dependency(TaskId(40), TaskId(41), Dependency::DependsOn)
            .unwrap();

        let problem = SchedulingProblem::from_blocks(vec![block]).unwrap();

        let ps = windows_set(&[(0.0, 3.0)]);
        let mut periods: TaskPeriodMap = HashMap::new();
        periods.insert(TaskId(40), ps.clone());
        periods.insert(TaskId(41), ps);

        let schedule = Schedule::new();
        let ctx = ctx_with(0.0, 0.0, 4.0, &periods);
        let score = fom.evaluate(&schedule, &problem, &ctx);

        // Only task 40 is recoverable → recoverable_count = 1 → score ≈ 10.
        assert!(
            score < 20.0,
            "successor must not count when predecessor is unscheduled; score={score}"
        );
    }

    #[test]
    fn no_context_falls_back_to_placed_count() {
        let fom = FutureFlexibilityFom;
        let task = make_task(50, 1.0);
        let problem = SchedulingProblem::from_blocks(vec![
            SchedulingBlock::from_tasks(SchedulingBlockId(50), vec![task]).unwrap(),
        ])
        .unwrap();

        let mut schedule = Schedule::new();
        place(&mut schedule, 50, 0.0, 1.0);

        let ctx = FomContext::single_cursor(t(1.0), period(0.0, 5.0), None);
        let score = fom.evaluate(&schedule, &problem, &ctx);
        assert!(
            (score - 1.0).abs() < 1e-9,
            "no context → fall back to placed count (1); got {score}"
        );
    }

    #[test]
    fn soft_term_increases_score_for_high_priority_placed_tasks() {
        let fom = FutureFlexibilityFom;
        let high = make_task_with_priority(60, 1.0, 1.0);
        let low = make_task_with_priority(61, 1.0, 0.0);

        let prob_high = SchedulingProblem::from_blocks(vec![
            SchedulingBlock::from_tasks(SchedulingBlockId(60), vec![high]).unwrap(),
        ])
        .unwrap();
        let prob_low = SchedulingProblem::from_blocks(vec![
            SchedulingBlock::from_tasks(SchedulingBlockId(61), vec![low]).unwrap(),
        ])
        .unwrap();

        let periods: TaskPeriodMap = HashMap::new();
        let mut sched_high = Schedule::new();
        let mut sched_low = Schedule::new();
        place(&mut sched_high, 60, 0.0, 1.0);
        place(&mut sched_low, 61, 0.0, 1.0);

        let score_h = fom.evaluate(&sched_high, &prob_high, &ctx_with(1.0, 0.0, 5.0, &periods));
        let score_l = fom.evaluate(&sched_low, &prob_low, &ctx_with(1.0, 0.0, 5.0, &periods));

        assert!(
            score_h > score_l,
            "high-priority placed task must yield a higher soft_term; \
             score_h={score_h}, score_l={score_l}"
        );
    }
}
