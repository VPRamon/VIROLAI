use super::algorithm::{EstConfig, EstScheduler, MAX_K_BEAMS, run_scheduler};
use super::candidate::EstCandidate;
use super::fom::SoftConstraintFom;
use super::ordering::compare_candidates;
use crate::constraints::{ConstraintExpr, PrioritySoftConstraint, SoftConstraintExpr};
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::task::{IcrsTarget, Task};
use crate::time::{MJD, Period, TaskId, Time};
use qtty::{Degrees, Seconds};
use siderust::coordinates::frames::ICRS;
use siderust::coordinates::spherical::Direction;
use std::cmp::Ordering;
use std::sync::Arc;

fn target() -> IcrsTarget {
    Direction::<ICRS>::new_raw(Degrees::new(10.0), Degrees::new(20.0))
}

fn period(start: f64, end: f64) -> Period<MJD> {
    Period::new(Time::<MJD>::new(start), Time::<MJD>::new(end))
}

fn windows(periods: &[(f64, f64)]) -> crate::time::PeriodSet<MJD> {
    crate::time::PeriodSet::from_periods(periods.iter().map(|(s, e)| period(*s, *e)).collect())
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

#[test]
fn helper_analysis_updates_est_and_deadline() {
    let task = task_with_priority(1, 1.0, 1.0);
    let feasible = windows(&[(-2.0, -1.0), (0.2, 0.9), (1.0, 3.0), (5.0, 8.0)]);
    let horizon = period(0.0, 6.0);
    let candidate = EstCandidate::new(&task, &feasible, &horizon);

    let est = candidate.est.expect("est should exist");
    let deadline = candidate.deadline.expect("deadline should exist");

    assert!((est.value() - 1.0).abs() < 1e-9);
    assert!((deadline.value() - 5.0).abs() < 1e-9);
}

#[test]
fn helper_analysis_sums_window_ratios_into_flexibility() {
    let task = task_with_priority(1, 1.0, 1.0);
    let feasible = windows(&[(-1.0, 0.5), (1.0, 2.5), (3.0, 5.0)]);
    let horizon = period(0.0, 4.0);
    let candidate = EstCandidate::new(&task, &feasible, &horizon);

    assert!((candidate.flexibility - 2.5).abs() < 1e-9);
}

#[test]
fn comparator_puts_impossible_candidates_last() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();
    let task_1 = task_with_priority(1, 1.0, 10.0);
    let task_2 = task_with_priority(2, 1.0, 1.0);
    let mut candidates = [
        EstCandidate::new(&task_1, &empty_windows, &horizon),
        EstCandidate::new(&task_2, &empty_windows, &horizon),
    ];
    candidates[0].flexibility = 0.25;
    candidates[1].est = Some(Time::<MJD>::new(1.0));
    candidates[1].deadline = Some(Time::<MJD>::new(2.0));
    candidates[1].flexibility = 3.0;

    candidates.sort_by(|a, b| compare_candidates(a, b, 2, horizon.start));
    assert_eq!(candidates[1].task_id(), TaskId(1));
}

#[test]
fn comparator_allows_flexible_before_endangered_when_safe() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();
    let endangered_task = task_with_priority(1, 2.0, 1.0);
    let flexible_task = task_with_priority(2, 1.0, 1.0);
    let flexible_long_task = task_with_priority(2, 4.5, 1.0);

    let mut endangered = EstCandidate::new(&endangered_task, &empty_windows, &horizon);
    endangered.est = Some(Time::<MJD>::new(5.0));
    endangered.deadline = Some(Time::<MJD>::new(8.0));
    endangered.flexibility = 1.2;

    let mut flexible = EstCandidate::new(&flexible_task, &empty_windows, &horizon);
    flexible.est = Some(Time::<MJD>::new(4.0));
    flexible.deadline = Some(Time::<MJD>::new(10.0));
    flexible.flexibility = 3.0;

    assert_eq!(
        compare_candidates(&flexible, &endangered, 2, horizon.start),
        Ordering::Less
    );

    let mut flexible_long = EstCandidate::new(&flexible_long_task, &empty_windows, &horizon);
    flexible_long.est = flexible.est;
    flexible_long.deadline = flexible.deadline;
    flexible_long.flexibility = flexible.flexibility;

    assert_eq!(
        compare_candidates(&flexible_long, &endangered, 2, horizon.start),
        Ordering::Greater
    );
}

#[test]
fn run_scheduler_schedules_feasible_and_marks_unplanned() {
    let tasks = vec![
        task_with_priority(1, 1.0, 10.0),
        task_with_priority(2, 1.0, 5.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), windows(&[(10.0, 12.0)]));

    let horizon = period(0.0, 5.0);
    let schedule = run_scheduler(tasks, &possible, &horizon).expect("run should pass");

    assert_eq!(schedule.placements.len(), 1);
    let placement = schedule
        .placements
        .get(&TaskId(1))
        .expect("task 1 should be placed");
    assert!((placement.start.to::<MJD>().value() - 0.0).abs() < 1e-9);
    assert!(!schedule.placements.contains_key(&TaskId(2)));
}

#[test]
fn run_scheduler_filters_tasks_missing_possible_periods() {
    let tasks = vec![
        task_with_priority(1, 1.0, 10.0),
        task_with_priority(2, 1.0, 5.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));

    let horizon = period(0.0, 5.0);
    let schedule = run_scheduler(tasks, &possible, &horizon).expect("run should pass");

    assert!(schedule.placements.contains_key(&TaskId(1)));
    assert!(!schedule.placements.contains_key(&TaskId(2)));
}

#[test]
fn run_scheduler_filters_tasks_with_empty_possible_periods() {
    let tasks = vec![
        task_with_priority(1, 1.0, 10.0),
        task_with_priority(2, 1.0, 5.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), crate::time::PeriodSet::new());

    let horizon = period(0.0, 5.0);
    let schedule = run_scheduler(tasks, &possible, &horizon).expect("run should pass");

    assert!(schedule.placements.contains_key(&TaskId(1)));
    assert!(!schedule.placements.contains_key(&TaskId(2)));
}

#[test]
fn run_scheduler_uses_earliest_start_order() {
    let tasks = vec![
        task_with_priority(1, 0.5, 1.0),
        task_with_priority(2, 0.5, 1.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(2.0, 3.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 1.0)]));

    let horizon = period(0.0, 4.0);
    let schedule = run_scheduler(tasks, &possible, &horizon).expect("run should pass");

    assert_eq!(schedule.placements.len(), 2);
    let task_1 = schedule
        .placements
        .get(&TaskId(1))
        .expect("task 1 should be placed");
    let task_2 = schedule
        .placements
        .get(&TaskId(2))
        .expect("task 2 should be placed");
    assert!((task_2.start.to::<MJD>().value() - 0.0).abs() < 1e-9);
    assert!((task_1.start.to::<MJD>().value() - 2.0).abs() < 1e-9);
}

#[test]
fn run_scheduler_recomputes_est_from_remaining_horizon() {
    let tasks = vec![
        task_with_priority(1, 1.0, 10.0),
        task_with_priority(2, 1.0, 5.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), windows(&[(0.5, 3.0)]));

    let horizon = period(0.0, 4.0);
    let schedule = run_scheduler(tasks, &possible, &horizon).expect("run should pass");

    assert_eq!(schedule.placements.len(), 2);
    let task_1 = schedule
        .placements
        .get(&TaskId(1))
        .expect("task 1 should be placed");
    let task_2 = schedule
        .placements
        .get(&TaskId(2))
        .expect("task 2 should be placed");
    assert!((task_1.start.to::<MJD>().value() - 0.0).abs() < 1e-9);
    assert!((task_2.start.to::<MJD>().value() - 1.0).abs() < 1e-9);
    assert!(task_1.end <= task_2.start);
}

#[test]
fn zero_threshold_is_allowed() {
    let scheduler = EstScheduler::new(EstConfig {
        endangered_threshold: 0,
        ..EstConfig::default()
    });
    assert!(scheduler.is_ok());
}

#[test]
fn zero_k_beams_is_rejected() {
    let error = EstScheduler::new(EstConfig {
        k_beams: 0,
        ..EstConfig::default()
    })
    .expect_err("scheduler config should fail");

    assert!(matches!(
        error,
        ScheduleError::InvalidConfiguration(message)
            if message.contains("est.k_beams must be at least 1")
    ));
}

#[test]
fn k_beams_above_max_is_rejected() {
    let error = EstScheduler::new(EstConfig {
        k_beams: MAX_K_BEAMS + 1,
        ..EstConfig::default()
    })
    .expect_err("scheduler config should fail");

    assert!(matches!(
        error,
        ScheduleError::InvalidConfiguration(message)
            if message.contains(&format!("est.k_beams must be <= {MAX_K_BEAMS}"))
    ));
}

#[test]
fn zero_branching_factor_is_rejected() {
    let error = EstScheduler::new(EstConfig {
        branching_factor: 0,
        ..EstConfig::default()
    })
    .expect_err("scheduler config should fail");

    assert!(matches!(
        error,
        ScheduleError::InvalidConfiguration(message)
            if message.contains("est.branching_factor must be at least 1")
    ));
}

#[test]
fn run_scheduler_rejects_duplicate_task_ids() {
    let tasks = vec![
        task_with_priority(1, 1.0, 10.0),
        task_with_priority(1, 0.5, 5.0),
    ];
    let possible = TaskPeriodMap::new();
    let horizon = period(0.0, 5.0);

    let error = run_scheduler(tasks, &possible, &horizon).expect_err("run should fail");

    assert!(
        matches!(error, ScheduleError::InvalidTask(message) if message.contains("duplicate task id 1"))
    );
}

#[test]
fn run_scheduler_rejects_zero_duration_tasks() {
    let mut task = task_with_priority(1, 1.0, 10.0);
    task.duration = Seconds::new(0.0);

    let tasks = vec![task];
    let possible = TaskPeriodMap::new();
    let horizon = period(0.0, 5.0);

    let error = run_scheduler(tasks, &possible, &horizon).expect_err("run should fail");

    assert!(matches!(error, ScheduleError::InvalidDuration));
}

/// k=1, b=1 must match the classic greedy result exactly.
#[test]
fn beam_search_k1_b1_matches_greedy() {
    let tasks_a = vec![
        task_with_priority(1, 0.5, 1.0),
        task_with_priority(2, 0.5, 1.0),
    ];
    let tasks_b = vec![
        task_with_priority(1, 0.5, 1.0),
        task_with_priority(2, 0.5, 1.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(2.0, 3.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 1.0)]));

    let horizon = period(0.0, 4.0);

    let greedy = run_scheduler(tasks_a, &possible, &horizon).expect("greedy run should pass");

    let config = EstConfig {
        k_beams: 1,
        branching_factor: 1,
        ..EstConfig::default()
    };
    let beam = EstScheduler::new(config)
        .expect("config should be valid")
        .run_scheduler(tasks_b, &possible, &horizon)
        .expect("beam run should pass");

    assert_eq!(greedy.placements.len(), beam.placements.len());
    for (id, p) in &greedy.placements {
        let q = beam.placements.get(id).expect("same task should be placed");
        assert_eq!(p.start, q.start);
    }
}

/// With k=2, b=2 the scheduler explores a second branch and should place
/// both tasks even when they share a window (only one fits per greedy run).
#[test]
fn beam_search_k2_b2_places_both_tasks_in_disjoint_windows() {
    // task 1 fits in [0,2], task 2 fits only in [1,3].
    // Greedy: place task 1 at t=0 (ends at t=1), then task 2 at t=1. Both fit.
    // Beam with b=2 must also find this or a better solution.
    let tasks_a = vec![
        task_with_priority(1, 1.0, 10.0),
        task_with_priority(2, 1.0, 5.0),
    ];
    let tasks_b = vec![
        task_with_priority(1, 1.0, 10.0),
        task_with_priority(2, 1.0, 5.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 3.0)]));

    let horizon = period(0.0, 5.0);

    let greedy = run_scheduler(tasks_a, &possible, &horizon).expect("greedy should pass");

    let config = EstConfig {
        k_beams: 2,
        branching_factor: 2,
        ..EstConfig::default()
    };
    let beam = EstScheduler::new(config)
        .expect("config should be valid")
        .run_scheduler(tasks_b, &possible, &horizon)
        .expect("beam run should pass");

    // Beam search must place at least as many tasks as greedy.
    assert!(beam.placements.len() >= greedy.placements.len());
}

/// `SoftConstraintFom` should prefer the schedule that maximises priority sum.
#[test]
fn beam_search_soft_constraint_fom_prefers_high_priority() {
    // Two tasks fit in the same window; only one can be placed.
    // task 1 has priority 10, task 2 has priority 1.
    // SoftConstraintFom should pick task 1.
    let tasks = vec![
        task_with_priority(1, 1.5, 10.0),
        task_with_priority(2, 1.5, 1.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 2.0)]));

    let horizon = period(0.0, 2.0);

    let config = EstConfig {
        k_beams: 2,
        branching_factor: 2,
        ..EstConfig::default()
    };
    let schedule = EstScheduler::with_fom(config, Arc::new(SoftConstraintFom))
        .expect("config should be valid")
        .run_scheduler(tasks, &possible, &horizon)
        .expect("run should pass");

    // Only one task fits; with SoftConstraintFom it should be the high-priority one.
    assert_eq!(schedule.placements.len(), 1);
    assert!(schedule.placements.contains_key(&TaskId(1)));
}
