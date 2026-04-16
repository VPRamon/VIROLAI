//! Integration tests for the scheduler.
//!
//! Tests are layered:
//! 1. Unit tests for `Task`, `SchedulingBlock`, `ConstraintExpr`.
//! 2. Integration tests for `Schedule::place_task`, overlap rejection,
//!    dependency ordering, and constraint violations.

use qtty::{Degrees, Meter, Quantity, Seconds};
use scheduler::{
    Period,
    constraints::{Constraint, ConstraintExpr, TimeWindowConstraint},
    error::ScheduleError,
    schedule::Schedule,
    scheduling_block::{Dependency, SchedulingBlock},
    task::Task,
    time::{SchedulingBlockId, TaskId, TimeInterval},
};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::{JulianDate, MJD};

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Roque de los Muchachos observatory (La Palma, Spain).
fn roque() -> Geodetic<ECEF> {
    Geodetic::<ECEF>::new(
        Degrees::new(-17.892),
        Degrees::new(28.762),
        Quantity::<Meter>::new(2396.0),
    )
}

/// A simple ICRS target pointing to Sirius (RA=101.287°, Dec=-16.716°).
fn sirius_target() -> scheduler::task::IcrsTarget {
    use siderust::coordinates::frames::ICRS;
    use siderust::coordinates::spherical::Direction;
    // Direction fields: polar = dec, azimuth = ra
    Direction::<ICRS>::new_raw(
        Degrees::new(-16.716), // polar = declination
        Degrees::new(101.287), // azimuth = right ascension
    )
}

/// Build a trivial no-constraints task with the given duration in seconds.
fn make_task(id: u64, duration_secs: f64) -> Task {
    Task::new(
        TaskId(id),
        format!("task-{id}"),
        sirius_target(),
        Seconds::new(duration_secs),
        ConstraintExpr::Intersection(vec![]),
        None,
    )
    .expect("task construction must succeed")
}

/// Julian Date interval with `start` and a given duration in seconds.
fn interval_at(start_jd: f64, duration_secs: f64) -> TimeInterval {
    let duration_days = duration_secs / 86_400.0;
    TimeInterval::new(
        JulianDate::new(start_jd),
        JulianDate::new(start_jd + duration_days),
    )
}

// ─── Task unit tests ──────────────────────────────────────────────────────────

#[test]
fn task_rejects_empty_name() {
    let result = Task::new(
        TaskId(1),
        "",
        sirius_target(),
        Seconds::new(600.0),
        ConstraintExpr::Intersection(vec![]),
        None,
    );
    assert!(matches!(result, Err(ScheduleError::InvalidTask(_))));
}

#[test]
fn task_rejects_zero_duration() {
    let result = Task::new(
        TaskId(1),
        "obs",
        sirius_target(),
        Seconds::new(0.0),
        ConstraintExpr::Intersection(vec![]),
        None,
    );
    assert!(matches!(result, Err(ScheduleError::InvalidDuration)));
}

#[test]
fn task_rejects_negative_duration() {
    let result = Task::new(
        TaskId(1),
        "obs",
        sirius_target(),
        Seconds::new(-1.0),
        ConstraintExpr::Intersection(vec![]),
        None,
    );
    assert!(matches!(result, Err(ScheduleError::InvalidDuration)));
}

#[test]
fn task_constructs_successfully() {
    let task = make_task(42, 900.0);
    assert_eq!(task.id, TaskId(42));
    assert_eq!(task.duration.value(), 900.0);
}

// ─── SchedulingBlock unit tests ───────────────────────────────────────────────

#[test]
fn block_detects_cycle() {
    let mut block = SchedulingBlock::new(SchedulingBlockId(1));
    let a = TaskId(1);
    let b = TaskId(2);
    block.add_task(a);
    block.add_task(b);
    block.add_dependency(a, b, Dependency::DependsOn).unwrap();
    // Adding b → a would close the cycle.
    let result = block.add_dependency(b, a, Dependency::DependsOn);
    assert!(matches!(result, Err(ScheduleError::DependencyCycle)));
}

#[test]
fn block_topological_order() {
    let mut block = SchedulingBlock::new(SchedulingBlockId(1));
    let a = TaskId(10);
    let b = TaskId(20);
    let c = TaskId(30);
    block.add_task(a);
    block.add_task(b);
    block.add_task(c);
    // c depends on b, b depends on a → expected order: a, b, c
    block.add_dependency(b, a, Dependency::DependsOn).unwrap();
    block.add_dependency(c, b, Dependency::DependsOn).unwrap();
    let order = block.topological_order().unwrap();
    let pos_a = order.iter().position(|&t| t == a).unwrap();
    let pos_b = order.iter().position(|&t| t == b).unwrap();
    let pos_c = order.iter().position(|&t| t == c).unwrap();
    assert!(pos_a < pos_b, "a must come before b");
    assert!(pos_b < pos_c, "b must come before c");
}

// ─── Constraint unit tests ────────────────────────────────────────────────────

#[test]
fn time_window_constraint_accepts_inside() {
    let site = roque();
    // Task only provides target for the optional check argument.
    let task_for_ctx = make_task(1, 600.0);
    let window = interval_at(2_460_000.0, 7200.0);
    let candidate = interval_at(2_460_000.0 + 100.0 / 86_400.0, 600.0);

    let timeline = Period::<MJD>::new(candidate.start.to::<MJD>(), candidate.end.to::<MJD>());
    let c = TimeWindowConstraint {
        window: Period::<MJD>::new(window.start.to::<MJD>(), window.end.to::<MJD>()),
    };
    assert!(
        !c.check(&timeline, Some(&site), Some(&task_for_ctx.target))
            .is_empty()
    );
}

#[test]
fn time_window_constraint_rejects_outside() {
    let site = roque();
    let task_for_ctx = make_task(1, 600.0);
    // window ends before candidate
    let window = interval_at(2_460_000.0, 600.0);
    let candidate = interval_at(2_460_000.0 + 700.0 / 86_400.0, 600.0);

    let timeline = Period::<MJD>::new(candidate.start.to::<MJD>(), candidate.end.to::<MJD>());
    let c = TimeWindowConstraint {
        window: Period::<MJD>::new(window.start.to::<MJD>(), window.end.to::<MJD>()),
    };
    let periods = c.check(&timeline, Some(&site), Some(&task_for_ctx.target));
    assert!(periods.is_empty());
}

// ─── Schedule integration tests ───────────────────────────────────────────────

#[test]
fn place_valid_task() {
    let mut schedule = Schedule::new();
    let task = make_task(1, 600.0);
    schedule.add_task(task);

    let site = roque();
    let interval = interval_at(2_460_000.0, 600.0);
    schedule
        .place_task(TaskId(1), interval, None, &site)
        .unwrap();

    assert!(schedule.placements.contains_key(&TaskId(1)));
}

#[test]
fn reject_overlap() {
    let mut schedule = Schedule::new();
    schedule.add_task(make_task(1, 600.0));
    schedule.add_task(make_task(2, 600.0));

    let site = roque();
    let i1 = interval_at(2_460_000.0, 600.0);
    let i2 = interval_at(2_460_000.0 + 300.0 / 86_400.0, 600.0); // overlaps i1

    schedule.place_task(TaskId(1), i1, None, &site).unwrap();
    let result = schedule.place_task(TaskId(2), i2, None, &site);
    assert!(matches!(result, Err(ScheduleError::OverlapConflict)));
}

#[test]
fn reject_duration_mismatch() {
    let mut schedule = Schedule::new();
    schedule.add_task(make_task(1, 600.0));

    let site = roque();
    // Give it 300 s instead of the required 600 s.
    let short = interval_at(2_460_000.0, 300.0);
    let result = schedule.place_task(TaskId(1), short, None, &site);
    assert!(matches!(
        result,
        Err(ScheduleError::IntervalDurationMismatch)
    ));
}

#[test]
fn reject_unknown_task() {
    let mut schedule = Schedule::new();
    let site = roque();
    let interval = interval_at(2_460_000.0, 600.0);
    let result = schedule.place_task(TaskId(99), interval, None, &site);
    assert!(matches!(result, Err(ScheduleError::TaskNotFound)));
}

#[test]
fn unplace_task() {
    let mut schedule = Schedule::new();
    schedule.add_task(make_task(1, 600.0));

    let site = roque();
    let interval = interval_at(2_460_000.0, 600.0);
    schedule
        .place_task(TaskId(1), interval, None, &site)
        .unwrap();
    schedule.unplace_task(TaskId(1)).unwrap();

    assert!(!schedule.placements.contains_key(&TaskId(1)));
}

#[test]
fn move_task() {
    let mut schedule = Schedule::new();
    schedule.add_task(make_task(1, 600.0));

    let site = roque();
    let i1 = interval_at(2_460_000.0, 600.0);
    let i2 = interval_at(2_460_000.0 + 3600.0 / 86_400.0, 600.0); // 1 hour later

    schedule.place_task(TaskId(1), i1, None, &site).unwrap();
    schedule.move_task(TaskId(1), i2, None, &site).unwrap();

    let p = schedule.placements.get(&TaskId(1)).unwrap();
    assert!((p.start.value() - i2.start.value()).abs() < 1e-9);
}

#[test]
fn block_dependency_ordering_respected() {
    let mut schedule = Schedule::new();
    schedule.add_task(make_task(1, 600.0));
    schedule.add_task(make_task(2, 600.0));

    let mut block = SchedulingBlock::new(SchedulingBlockId(10));
    block.add_task(TaskId(1));
    block.add_task(TaskId(2));
    // task 2 depends on task 1: place task 1 first
    block
        .add_dependency(TaskId(2), TaskId(1), Dependency::DependsOn)
        .unwrap();
    schedule.add_block(block);

    let site = roque();
    let i1 = interval_at(2_460_000.0, 600.0);
    let i2 = interval_at(2_460_000.0 + 3600.0 / 86_400.0, 600.0);

    // Place task 1 first, then task 2 — should succeed.
    schedule
        .place_task(TaskId(1), i1, Some(SchedulingBlockId(10)), &site)
        .unwrap();
    schedule
        .place_task(TaskId(2), i2, Some(SchedulingBlockId(10)), &site)
        .unwrap();
}

#[test]
fn block_dependency_ordering_rejected_when_predecessor_unplaced() {
    let mut schedule = Schedule::new();
    schedule.add_task(make_task(1, 600.0));
    schedule.add_task(make_task(2, 600.0));

    let mut block = SchedulingBlock::new(SchedulingBlockId(10));
    block.add_task(TaskId(1));
    block.add_task(TaskId(2));
    // task 2 depends on task 1
    block
        .add_dependency(TaskId(2), TaskId(1), Dependency::DependsOn)
        .unwrap();
    schedule.add_block(block);

    let site = roque();
    let i2 = interval_at(2_460_000.0 + 3600.0 / 86_400.0, 600.0);

    // Try to place task 2 before task 1 — should fail.
    let result = schedule.place_task(TaskId(2), i2, Some(SchedulingBlockId(10)), &site);
    assert!(matches!(result, Err(ScheduleError::ConstraintViolation(_))));
}
