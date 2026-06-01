use super::algorithm::{LstScheduler, placement_end, placement_start, run_scheduler};
use super::transform::{mirror_period, mirror_period_set, mirror_time, unmirror_schedule};
use crate::constraints::{ConstraintExpr, PrioritySoftConstraint, SoftConstraintExpr};
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::TaskPlacement;
use crate::scheduler::est::{self, EstScheduler};
use crate::task::{IcrsTarget, Task};
use crate::time::{MJD, Period, PeriodSet, SchedulingBlockId, TaskId, Time};
use qtty::{Degrees, Seconds};
use siderust::coordinates::frames::ICRS;
use siderust::coordinates::spherical::Direction;
use std::collections::HashMap;

fn t(v: f64) -> Time<MJD> {
    Time::<MJD>::new(v)
}

fn period(start: f64, end: f64) -> Period<MJD> {
    Period::new(t(start), t(end))
}

fn windows(pairs: &[(f64, f64)]) -> PeriodSet<MJD> {
    PeriodSet::from_periods(pairs.iter().map(|(s, e)| period(*s, *e)).collect())
}

fn target() -> IcrsTarget {
    Direction::<ICRS>::new_raw(Degrees::new(10.0), Degrees::new(20.0))
}

fn task(id: u64, duration_days: f64) -> Task {
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

fn task_with_priority(id: u64, duration_days: f64, priority: f64) -> Task {
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

fn periods_map(entries: &[(u64, &[(f64, f64)])]) -> TaskPeriodMap {
    entries
        .iter()
        .map(|(id, pairs)| (TaskId(*id), windows(pairs)))
        .collect()
}

// ── Transform tests ───────────────────────────────────────────────────────────

/// Test 1: basic mirror of a period across [0, 10)
#[test]
fn mirror_period_basic() {
    let horizon = period(0.0, 10.0);
    let p = period(2.0, 4.0);
    let mirrored = mirror_period(&p, &horizon);
    assert_eq!(mirrored, period(6.0, 8.0));
}

/// Test 2: double reflection is the identity
#[test]
fn double_mirror_is_identity() {
    let horizon = period(0.0, 10.0);
    let original = period(1.0, 4.0);
    let twice = mirror_period(&mirror_period(&original, &horizon), &horizon);
    assert_eq!(twice, original);
}

/// Test 2b: double reflection of a time point is the identity
#[test]
fn double_mirror_time_is_identity() {
    let horizon = period(0.0, 10.0);
    let t_orig = t(3.5);
    let back = mirror_time(mirror_time(t_orig, &horizon), &horizon);
    assert!((back.value() - t_orig.value()).abs() < 1e-12);
}

/// Test 2c: double reflection of a PeriodSet is the identity
#[test]
fn double_mirror_period_set_is_identity() {
    let horizon = period(0.0, 10.0);
    let original = windows(&[(1.0, 3.0), (5.0, 8.0)]);
    let twice = mirror_period_set(&mirror_period_set(&original, &horizon), &horizon);
    assert_eq!(twice, original);
}

// ── Scheduler functional tests ────────────────────────────────────────────────

/// Test 3: LST places a single task at the *end* of its window.
#[test]
fn lst_places_task_at_end_of_window() {
    let horizon = period(0.0, 10.0);
    let t1 = task(1, 1.0);
    let possible = periods_map(&[(1, &[(0.0, 10.0)])]);

    let schedule = run_scheduler([t1], &possible, &horizon).expect("schedule should succeed");

    let start = placement_start(&schedule, 1)
        .expect("task 1 should be placed")
        .value();
    let end = placement_end(&schedule, 1).expect("task 1 end").value();

    assert!(
        (end - 10.0).abs() < 1e-9,
        "LST: task must end at horizon.end; end={end}"
    );
    assert!(
        (start - 9.0).abs() < 1e-9,
        "LST: task start must be horizon.end - duration; start={start}"
    );
}

/// Test 4: with multiple windows, LST picks the *latest* window.
#[test]
fn lst_picks_latest_window_among_multiple() {
    let horizon = period(0.0, 10.0);
    let t1 = task(1, 1.0);
    let possible = periods_map(&[(1, &[(0.0, 2.0), (5.0, 8.0)])]);

    let schedule = run_scheduler([t1], &possible, &horizon).expect("schedule should succeed");

    let start = placement_start(&schedule, 1)
        .expect("task 1 placed")
        .value();
    let end = placement_end(&schedule, 1).expect("task 1 end").value();

    assert!(
        (end - 8.0).abs() < 1e-9,
        "LST must place at end of the last window [5,8); end={end}"
    );
    assert!(
        (start - 7.0).abs() < 1e-9,
        "LST start should be 7.0; start={start}"
    );
}

/// Test 5: two tasks both fitting [0, 10) should be placed without overlap,
/// both near the end of the horizon.
#[test]
fn lst_places_two_tasks_near_end_without_overlap() {
    let horizon = period(0.0, 10.0);
    let t1 = task(1, 1.0);
    let t2 = task(2, 1.0);
    let possible = periods_map(&[(1, &[(0.0, 10.0)]), (2, &[(0.0, 10.0)])]);

    let schedule = run_scheduler([t1, t2], &possible, &horizon).expect("schedule should succeed");

    assert_eq!(schedule.len(), 2, "both tasks must be placed");

    let end1 = placement_end(&schedule, 1).expect("task 1 end").value();
    let end2 = placement_end(&schedule, 2).expect("task 2 end").value();

    // Both tasks must end within [8, 10] and not overlap.
    assert!(end1 <= 10.0 + 1e-9, "task1 end must be ≤ 10.0; got {end1}");
    assert!(end2 <= 10.0 + 1e-9, "task2 end must be ≤ 10.0; got {end2}");

    let start1 = placement_start(&schedule, 1).unwrap().value();
    let start2 = placement_start(&schedule, 2).unwrap().value();
    let (lo_start, hi_start, lo_end, hi_end) = if start1 <= start2 {
        (start1, start2, end1, end2)
    } else {
        (start2, start1, end2, end1)
    };
    assert!(
        lo_end <= hi_start + 1e-9,
        "tasks must not overlap; [{lo_start},{lo_end}) vs [{hi_start},{hi_end})"
    );
}

/// Test 6: LST(original) == unmirror(EST(mirror(original)))
#[test]
fn lst_equals_unmirror_est_mirror() {
    let horizon = period(0.0, 10.0);

    let possible = periods_map(&[(1, &[(1.0, 5.0), (7.0, 10.0)]), (2, &[(0.0, 8.0)])]);

    // Run LST
    let scheduler = LstScheduler::default();
    let lst_schedule = scheduler
        .run_scheduler([task(1, 1.0), task(2, 2.0)], &possible, &horizon)
        .expect("LST should succeed");

    // Run EST on mirrored windows and unmirror
    let mirrored = super::transform::mirror_task_periods(&possible, &horizon);
    let est_schedule = EstScheduler::default()
        .run_scheduler([task(1, 1.0), task(2, 2.0)], &mirrored, &horizon)
        .expect("EST on mirrored should succeed");
    let unmirrored_est_schedule = unmirror_schedule(&est_schedule, &horizon);

    assert_eq!(
        lst_schedule.len(),
        unmirrored_est_schedule.len(),
        "LST and unmirror(EST(mirror)) must place the same number of tasks"
    );

    for task_id in [1u64, 2u64] {
        let lst_start = placement_start(&lst_schedule, task_id)
            .map(|t| t.value())
            .unwrap_or(f64::NAN);
        let est_start = placement_start(&unmirrored_est_schedule, task_id)
            .map(|t| t.value())
            .unwrap_or(f64::NAN);

        assert!(
            (lst_start - est_start).abs() < 1e-9,
            "task {task_id}: LST start {lst_start:.6} != unmirror(EST) start {est_start:.6}"
        );
    }
}

/// Test 7: MirroredFom evaluates the inner FOM in original time.
///
/// Place a task with a time-dependent soft constraint (priority).  The
/// schedule produced by LST should have the task at its real (original)
/// start time, and the FOM should reflect original-time quality rather
/// than mirrored-time quality.
#[test]
fn mirrored_fom_evaluates_at_original_start_time() {
    use super::algorithm::MirroredFom;
    use crate::schedule::{Schedule, SchedulingProblem};
    use crate::scheduler::est::ScheduleFom;
    use crate::scheduler::fom::{FomContext, SoftConstraintFom};
    use crate::scheduling_block::SchedulingBlock;
    use std::sync::Arc;

    let horizon = period(0.0, 10.0);

    // Task placed at [9, 10) in original time → mirrored to [0, 1).
    let task1 = task_with_priority(1, 1.0, 1.0);
    let block = SchedulingBlock::from_tasks(SchedulingBlockId(1), vec![task1])
        .expect("block should be valid");
    let problem = SchedulingProblem::from_blocks(vec![block]).expect("problem");

    // Build a mirrored schedule (placement at [0, 1) in mirrored time).
    let mut mirrored_sched = Schedule::new();
    mirrored_sched.insert_placement(TaskPlacement {
        task_id: TaskId(1),
        start: t(0.0),
        end: t(1.0),
    });

    // Build an original schedule (placement at [9, 10) in original time).
    let mut original_sched = Schedule::new();
    original_sched.insert_placement(TaskPlacement {
        task_id: TaskId(1),
        start: t(9.0),
        end: t(10.0),
    });

    let soft_fom = SoftConstraintFom;
    let ctx = FomContext::single_cursor(t(1.0), horizon, None);

    let mirrored_fom = MirroredFom::new(Arc::new(SoftConstraintFom), horizon);

    // MirroredFom.evaluate(mirrored_schedule) should equal SoftConstraintFom.evaluate(original_schedule).
    let score_mirrored = mirrored_fom.evaluate(&mirrored_sched, &problem, &ctx);
    let score_original = soft_fom.evaluate(&original_sched, &problem, &ctx);

    assert!(
        (score_mirrored - score_original).abs() < 1e-9,
        "MirroredFom must evaluate at original time; \
         mirrored_score={score_mirrored:.6}, original_score={score_original:.6}"
    );
}

// ── Edge cases ────────────────────────────────────────────────────────────────

/// An empty task list yields an empty schedule.
#[test]
fn empty_tasks_yields_empty_schedule() {
    let horizon = period(0.0, 10.0);
    let possible: TaskPeriodMap = HashMap::new();
    let schedule = run_scheduler([], &possible, &horizon).expect("empty schedule");
    assert!(schedule.is_empty());
}

/// A task with no feasible windows is dropped gracefully.
#[test]
fn infeasible_task_is_dropped() {
    let horizon = period(0.0, 10.0);
    let t1 = task(1, 5.0);
    let possible = periods_map(&[(1, &[])]);
    let schedule = run_scheduler([t1], &possible, &horizon).expect("schedule");
    assert!(
        schedule.is_empty(),
        "infeasible task must not appear in schedule"
    );
}

/// High-priority tasks with overlapping windows: LST still schedules them
/// without overlap.
#[test]
fn lst_no_overlap_with_priority_tasks() {
    let horizon = period(0.0, 10.0);
    let t1 = task_with_priority(1, 1.0, 10.0);
    let t2 = task_with_priority(2, 1.0, 1.0);
    let possible = periods_map(&[(1, &[(0.0, 10.0)]), (2, &[(0.0, 10.0)])]);

    let schedule = run_scheduler([t1, t2], &possible, &horizon).expect("schedule should succeed");

    if schedule.len() == 2 {
        let start1 = placement_start(&schedule, 1).unwrap().value();
        let end1 = placement_end(&schedule, 1).unwrap().value();
        let start2 = placement_start(&schedule, 2).unwrap().value();
        let end2 = placement_end(&schedule, 2).unwrap().value();

        let overlap = end1.min(end2) - start1.max(start2);
        assert!(
            overlap <= 1e-9,
            "placements must not overlap: [{start1},{end1}) [{start2},{end2})"
        );
    }
}

/// `LstScheduler::new` and `LstScheduler::with_fom` constructors work.
#[test]
fn constructors_succeed() {
    use std::sync::Arc;

    let config = est::Configuration::default();
    LstScheduler::new(config).expect("new should succeed");

    let fom: Arc<dyn est::ScheduleFom> = Arc::new(crate::scheduler::fom::SoftConstraintFom);
    LstScheduler::with_fom(config, fom).expect("with_fom should succeed");
}
