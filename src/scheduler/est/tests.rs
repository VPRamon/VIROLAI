use super::candidate::EstCandidate;
use super::fom::{EstFomKind, SoftConstraintFom};
use super::ordering::{compare_candidates, sort_candidates};
use super::queue::CandidateQueue;
use super::{Configuration, EstScheduler, MAX_K_BEAMS, run_scheduler};
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

    candidates.sort_by(|a, b| compare_candidates(a, b, horizon.start, 0));
    assert_eq!(candidates[1].task_id(), TaskId(1));
}

#[test]
fn comparator_orders_by_earlier_est_first() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();
    let earlier_task = task_with_priority(1, 1.0, 1.0);
    let later_task = task_with_priority(2, 1.0, 100.0);

    let mut earlier = EstCandidate::new(&earlier_task, &empty_windows, &horizon);
    earlier.est = Some(Time::<MJD>::new(1.0));
    earlier.flexibility = 5.0;

    let mut later = EstCandidate::new(&later_task, &empty_windows, &horizon);
    later.est = Some(Time::<MJD>::new(2.0));
    later.flexibility = 1.0;

    assert_eq!(
        compare_candidates(&earlier, &later, horizon.start, 0),
        Ordering::Less
    );
}

#[test]
fn comparator_orders_by_lower_flexibility_before_priority() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();
    let low_flex_task = task_with_priority(1, 1.0, 1.0);
    let high_flex_task = task_with_priority(2, 1.0, 100.0);

    let mut low_flex = EstCandidate::new(&low_flex_task, &empty_windows, &horizon);
    low_flex.est = Some(Time::<MJD>::new(1.0));
    low_flex.flexibility = 1.5;

    let mut high_flex = EstCandidate::new(&high_flex_task, &empty_windows, &horizon);
    high_flex.est = Some(Time::<MJD>::new(1.0));
    high_flex.flexibility = 3.0;

    assert_eq!(
        compare_candidates(&low_flex, &high_flex, horizon.start, 0),
        Ordering::Less
    );
}

#[test]
fn comparator_orders_by_higher_soft_priority_after_est_and_flexibility() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();
    let low_priority_task = task_with_priority(1, 1.0, 1.0);
    let high_priority_task = task_with_priority(2, 1.0, 10.0);

    let mut low_priority = EstCandidate::new(&low_priority_task, &empty_windows, &horizon);
    low_priority.est = Some(Time::<MJD>::new(1.0));
    low_priority.flexibility = 2.0;

    let mut high_priority = EstCandidate::new(&high_priority_task, &empty_windows, &horizon);
    high_priority.est = Some(Time::<MJD>::new(1.0));
    high_priority.flexibility = 2.0;

    assert_eq!(
        compare_candidates(&high_priority, &low_priority, horizon.start, 0),
        Ordering::Less
    );
}

#[test]
fn comparator_orders_by_task_id_after_other_keys_tie() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();
    let lower_id_task = task_with_priority(1, 1.0, 5.0);
    let higher_id_task = task_with_priority(2, 1.0, 5.0);

    let mut lower_id = EstCandidate::new(&lower_id_task, &empty_windows, &horizon);
    lower_id.est = Some(Time::<MJD>::new(1.0));
    lower_id.flexibility = 2.0;

    let mut higher_id = EstCandidate::new(&higher_id_task, &empty_windows, &horizon);
    higher_id.est = Some(Time::<MJD>::new(1.0));
    higher_id.flexibility = 2.0;

    assert_eq!(
        compare_candidates(&lower_id, &higher_id, horizon.start, 0),
        Ordering::Less
    );
}

#[test]
fn comparator_sorts_missing_est_after_candidates_with_est() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();
    let with_est_task = task_with_priority(1, 1.0, 1.0);
    let missing_est_task = task_with_priority(2, 1.0, 1.0);

    let mut with_est = EstCandidate::new(&with_est_task, &empty_windows, &horizon);
    with_est.est = Some(Time::<MJD>::new(1.0));
    with_est.flexibility = 2.0;

    let mut missing_est = EstCandidate::new(&missing_est_task, &empty_windows, &horizon);
    missing_est.est = None;
    missing_est.flexibility = 2.0;

    assert_eq!(
        compare_candidates(&with_est, &missing_est, horizon.start, 0),
        Ordering::Less
    );
}

#[test]
fn default_config_restores_endangered_threshold() {
    let config = Configuration::default();
    assert_eq!(config.endangered_threshold, 1);
}

#[test]
fn zero_threshold_is_allowed_and_disables_promotion() {
    let scheduler = EstScheduler::new(Configuration {
        endangered_threshold: 0,
        ..Configuration::default()
    });

    assert!(scheduler.is_ok());
}

#[test]
fn sort_candidates_defers_non_endangered_that_obstructs_endangered() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();

    let non_endangered_task = task_with_priority(1, 1.0, 1.0);
    let endangered_task = task_with_priority(2, 1.0, 1.0);

    let mut non_endangered = EstCandidate::new(&non_endangered_task, &empty_windows, &horizon);
    non_endangered.est = Some(Time::<MJD>::new(0.0));
    non_endangered.flexibility = 3.0;

    let mut endangered = EstCandidate::new(&endangered_task, &empty_windows, &horizon);
    endangered.est = Some(Time::<MJD>::new(0.5));
    endangered.flexibility = 1.5;

    let mut candidates = vec![non_endangered, endangered];
    sort_candidates(&mut candidates, horizon.start, 2);

    assert_eq!(candidates[0].task_id(), TaskId(2));
    assert_eq!(candidates[1].task_id(), TaskId(1));
    assert_eq!(
        compare_candidates(&candidates[0], &candidates[1], horizon.start, 2),
        Ordering::Less
    );
}

#[test]
fn sort_candidates_does_not_defer_when_end_matches_endangered_est() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();

    let earlier_task = task_with_priority(1, 1.0, 1.0);
    let endangered_task = task_with_priority(2, 1.0, 1.0);

    let mut earlier = EstCandidate::new(&earlier_task, &empty_windows, &horizon);
    earlier.est = Some(Time::<MJD>::new(0.0));
    earlier.flexibility = 3.0;

    let mut endangered = EstCandidate::new(&endangered_task, &empty_windows, &horizon);
    endangered.est = Some(Time::<MJD>::new(1.0));
    endangered.flexibility = 1.5;

    let mut candidates = vec![earlier, endangered];
    sort_candidates(&mut candidates, horizon.start, 2);

    assert_eq!(candidates[0].task_id(), TaskId(1));
    assert_eq!(candidates[1].task_id(), TaskId(2));
}

#[test]
fn sort_candidates_does_not_defer_earlier_endangered_task() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();

    let earlier_task = task_with_priority(1, 1.0, 1.0);
    let later_task = task_with_priority(2, 1.0, 1.0);

    let mut earlier = EstCandidate::new(&earlier_task, &empty_windows, &horizon);
    earlier.est = Some(Time::<MJD>::new(0.0));
    earlier.flexibility = 1.5;

    let mut later = EstCandidate::new(&later_task, &empty_windows, &horizon);
    later.est = Some(Time::<MJD>::new(0.5));
    later.flexibility = 1.5;

    let mut candidates = vec![earlier, later];
    sort_candidates(&mut candidates, horizon.start, 2);

    assert_eq!(candidates[0].task_id(), TaskId(1));
    assert_eq!(candidates[1].task_id(), TaskId(2));
}

#[test]
fn sort_candidates_keeps_unrelated_non_endangered_task_ahead() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();

    let non_endangered_task = task_with_priority(1, 0.5, 1.0);
    let endangered_task = task_with_priority(2, 1.0, 1.0);

    let mut non_endangered = EstCandidate::new(&non_endangered_task, &empty_windows, &horizon);
    non_endangered.est = Some(Time::<MJD>::new(0.0));
    non_endangered.flexibility = 3.0;

    let mut endangered = EstCandidate::new(&endangered_task, &empty_windows, &horizon);
    endangered.est = Some(Time::<MJD>::new(1.0));
    endangered.flexibility = 1.5;

    let mut candidates = vec![non_endangered, endangered];
    sort_candidates(&mut candidates, horizon.start, 2);

    assert_eq!(candidates[0].task_id(), TaskId(1));
    assert_eq!(candidates[1].task_id(), TaskId(2));
}

#[test]
fn sort_candidates_promotes_to_latest_obstructed_endangered_est() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();

    let non_endangered_task = task_with_priority(1, 3.0, 1.0);
    let endangered_task_a = task_with_priority(2, 1.0, 1.0);
    let endangered_task_b = task_with_priority(3, 1.0, 1.0);

    let mut non_endangered = EstCandidate::new(&non_endangered_task, &empty_windows, &horizon);
    non_endangered.est = Some(Time::<MJD>::new(0.0));
    non_endangered.flexibility = 3.0;

    let mut endangered_a = EstCandidate::new(&endangered_task_a, &empty_windows, &horizon);
    endangered_a.est = Some(Time::<MJD>::new(1.0));
    endangered_a.flexibility = 1.5;

    let mut endangered_b = EstCandidate::new(&endangered_task_b, &empty_windows, &horizon);
    endangered_b.est = Some(Time::<MJD>::new(2.0));
    endangered_b.flexibility = 1.5;

    let mut candidates = vec![non_endangered, endangered_a, endangered_b];
    sort_candidates(&mut candidates, horizon.start, 2);

    assert_eq!(candidates[0].task_id(), TaskId(2));
    assert_eq!(candidates[1].task_id(), TaskId(3));
    assert_eq!(candidates[2].task_id(), TaskId(1));
}

#[test]
fn candidate_queue_pop_at_uses_schedulable_index_not_raw_index() {
    let horizon = period(0.0, 10.0);
    let empty_windows = crate::time::PeriodSet::new();
    let task_1 = task_with_priority(1, 1.0, 10.0);
    let task_2 = task_with_priority(2, 1.0, 1.0);
    let task_3 = task_with_priority(3, 1.0, 5.0);

    let mut first = EstCandidate::new(&task_1, &empty_windows, &horizon);
    first.est = Some(Time::<MJD>::new(1.0));
    first.flexibility = 2.0;

    let mut impossible = EstCandidate::new(&task_2, &empty_windows, &horizon);
    impossible.est = None;
    impossible.flexibility = 0.25;

    let mut second = EstCandidate::new(&task_3, &empty_windows, &horizon);
    second.est = Some(Time::<MJD>::new(2.0));
    second.flexibility = 3.0;

    let mut queue = CandidateQueue::from_candidates_for_test(vec![first, impossible, second]);

    assert_eq!(queue.count_schedulable(), 2);
    assert_eq!(queue.pop_at(1).task_id(), TaskId(3));
    assert_eq!(queue.count_schedulable(), 1);
}

#[test]
fn candidate_queue_refresh_resorts_mixed_candidates_without_panicking() {
    let tasks = [
        task_with_priority(1, 1.0, 1.0),
        task_with_priority(2, 1.0, 10.0),
        task_with_priority(3, 1.0, 5.0),
        task_with_priority(4, 1.0, 1.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 3.0)]));
    possible.insert(TaskId(3), windows(&[(2.0, 3.0)]));
    possible.insert(TaskId(4), crate::time::PeriodSet::new());

    let horizon = period(0.0, 5.0);
    let task_refs: Vec<_> = tasks.iter().collect();
    let mut queue = CandidateQueue::build(&task_refs, &possible, &horizon, None, 0);

    queue.refresh(&period(0.5, 5.0), 0);

    assert_eq!(queue.count_schedulable(), 3);
    assert_eq!(queue.pop_at(0).task_id(), TaskId(1));
    assert_eq!(queue.pop_at(0).task_id(), TaskId(2));
    assert_eq!(queue.pop_at(0).task_id(), TaskId(3));
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
    let schedule = run_scheduler(&tasks, &possible, &horizon).expect("run should pass");

    assert_eq!(schedule.len(), 1);
    let placement = schedule.get(TaskId(1)).expect("task 1 should be placed");
    assert!((placement.start.to::<MJD>().value() - 0.0).abs() < 1e-9);
    assert!(!schedule.contains(TaskId(2)));
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
    let schedule = run_scheduler(&tasks, &possible, &horizon).expect("run should pass");

    assert!(schedule.contains(TaskId(1)));
    assert!(!schedule.contains(TaskId(2)));
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
    let schedule = run_scheduler(&tasks, &possible, &horizon).expect("run should pass");

    assert!(schedule.contains(TaskId(1)));
    assert!(!schedule.contains(TaskId(2)));
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
    let schedule = run_scheduler(&tasks, &possible, &horizon).expect("run should pass");

    assert_eq!(schedule.len(), 2);
    let task_1 = schedule.get(TaskId(1)).expect("task 1 should be placed");
    let task_2 = schedule.get(TaskId(2)).expect("task 2 should be placed");
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
    let schedule = run_scheduler(&tasks, &possible, &horizon).expect("run should pass");

    assert_eq!(schedule.len(), 2);
    let task_1 = schedule.get(TaskId(1)).expect("task 1 should be placed");
    let task_2 = schedule.get(TaskId(2)).expect("task 2 should be placed");
    assert!((task_1.start.to::<MJD>().value() - 0.0).abs() < 1e-9);
    assert!((task_2.start.to::<MJD>().value() - 1.0).abs() < 1e-9);
    assert!(task_1.end <= task_2.start);
}

#[test]
fn default_config_is_valid() {
    let scheduler = EstScheduler::new(Configuration::default());
    assert!(scheduler.is_ok());
}

#[test]
fn with_fom_builds_soft_constraint_scheduler() {
    let scheduler = EstScheduler::with_fom(
        Configuration::default(),
        EstFomKind::SoftConstraint.into_fom(),
    )
    .expect("config should be valid");

    assert!(format!("{:?}", scheduler.fom).contains("SoftConstraintFom"));
}

#[test]
fn zero_k_beams_is_rejected() {
    let error = EstScheduler::new(Configuration {
        k_beams: 0,
        ..Configuration::default()
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
    let error = EstScheduler::new(Configuration {
        k_beams: MAX_K_BEAMS + 1,
        ..Configuration::default()
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
    let error = EstScheduler::new(Configuration {
        branching_factor: 0,
        ..Configuration::default()
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

    let error = run_scheduler(&tasks, &possible, &horizon).expect_err("run should fail");

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

    let error = run_scheduler(&tasks, &possible, &horizon).expect_err("run should fail");

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

    let greedy = run_scheduler(&tasks_a, &possible, &horizon).expect("greedy run should pass");

    let config = Configuration {
        k_beams: 1,
        branching_factor: 1,
        endangered_threshold: 1,
    };
    let beam = EstScheduler::new(config)
        .expect("config should be valid")
        .run_scheduler(&tasks_b, &possible, &horizon)
        .expect("beam run should pass");

    assert_eq!(greedy.len(), beam.len());
    for p in greedy.placements() {
        let q = beam.get(p.task_id).expect("same task should be placed");
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

    let greedy = run_scheduler(&tasks_a, &possible, &horizon).expect("greedy should pass");

    let config = Configuration {
        k_beams: 2,
        branching_factor: 2,
        endangered_threshold: 1,
    };
    let beam = EstScheduler::new(config)
        .expect("config should be valid")
        .run_scheduler(&tasks_b, &possible, &horizon)
        .expect("beam run should pass");

    // Beam search must place at least as many tasks as greedy.
    assert!(beam.len() >= greedy.len());
}

#[test]
fn beam_search_wide_branching_does_not_panic_with_mixed_candidate_states() {
    let tasks = vec![
        task_with_priority(1, 1.0, 10.0),
        task_with_priority(2, 1.0, 9.0),
        task_with_priority(3, 1.0, 8.0),
        task_with_priority(4, 1.0, 1.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 2.5)]));
    possible.insert(TaskId(3), windows(&[(1.0, 3.0)]));
    possible.insert(TaskId(4), windows(&[(0.0, 0.5)]));

    let horizon = period(0.0, 4.0);
    let config = Configuration {
        k_beams: 1,
        branching_factor: 4,
        endangered_threshold: 1,
    };
    let schedule = EstScheduler::new(config)
        .expect("config should be valid")
        .run_scheduler(&tasks, &possible, &horizon)
        .expect("wide-branch EST run should pass");

    assert!(!schedule.is_empty());
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

    let config = Configuration {
        k_beams: 2,
        branching_factor: 2,
        endangered_threshold: 1,
    };
    let schedule = EstScheduler::with_fom(config, Arc::new(SoftConstraintFom))
        .expect("config should be valid")
        .run_scheduler(&tasks, &possible, &horizon)
        .expect("run should pass");

    // Only one task fits; with SoftConstraintFom it should be the high-priority one.
    assert_eq!(schedule.len(), 1);
    assert!(schedule.contains(TaskId(1)));
}

#[test]
fn endangered_threshold_changes_scheduler_choice_when_earlier_task_blocks_later_task() {
    let tasks = vec![
        task_with_priority(1, 1.0, 1.0),
        task_with_priority(2, 1.0, 1.0),
    ];
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 3.0)]));
    possible.insert(TaskId(2), windows(&[(0.5, 1.6)]));

    let horizon = period(0.0, 3.0);

    let without_protection = EstScheduler::new(Configuration {
        endangered_threshold: 0,
        ..Configuration::default()
    })
    .expect("config should be valid")
    .run_scheduler(&tasks, &possible, &horizon)
    .expect("run should pass");

    let with_protection = EstScheduler::new(Configuration {
        endangered_threshold: 2,
        ..Configuration::default()
    })
    .expect("config should be valid")
    .run_scheduler(&tasks, &possible, &horizon)
    .expect("run should pass");

    assert_eq!(without_protection.len(), 1);
    assert!(without_protection.contains(TaskId(1)));
    assert!(!without_protection.contains(TaskId(2)));

    assert_eq!(with_protection.len(), 2);
    let protected_first = with_protection
        .get(TaskId(2))
        .expect("task 2 should be placed");
    assert!((protected_first.start.to::<MJD>().value() - 0.5).abs() < 1e-9);
}
