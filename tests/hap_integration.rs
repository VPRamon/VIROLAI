//! Integration tests for the HAP scheduler.

use std::collections::HashMap;

use qtty::Seconds;
use scheduler::{
    Period, PeriodSet, TaskPeriodMap,
    constraints::ConstraintExpr,
    schedule::{Schedule, SchedulingProblem},
    scheduler::hap::{HapScheduler, Selector, SurvivorSelector, default_planner_config},
    scheduling_block::{Dependency, SchedulingBlock},
    task::Task,
    time::{MJD, SchedulingBlockId, TaskId, Time},
};
use siderust::coordinates::frames::ICRS;
use siderust::coordinates::spherical::Direction;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn horizon() -> Period<MJD> {
    // 1-day window
    Period::new(Time::<MJD>::new(60000.0), Time::<MJD>::new(60001.0))
}

fn make_task(id: u64, duration_secs: f64) -> Task {
    use qtty::Degrees;
    Task::new(
        TaskId(id),
        format!("task-{id}"),
        Direction::<ICRS>::new_raw(Degrees::new(-16.716), Degrees::new(101.287)),
        Seconds::new(duration_secs),
        ConstraintExpr::Intersection(vec![]),
        None,
    )
    .expect("task construction must succeed")
}

fn full_window(h: &Period<MJD>) -> PeriodSet<MJD> {
    PeriodSet::from_periods(vec![*h])
}

fn make_problem(blocks: Vec<SchedulingBlock>) -> SchedulingProblem {
    SchedulingProblem::from_blocks(blocks).unwrap()
}

fn block_from_task_ids(id: u64, task_ids: &[u64]) -> SchedulingBlock {
    SchedulingBlock::from_tasks(
        SchedulingBlockId(id),
        task_ids
            .iter()
            .map(|task_id| make_task(*task_id, 600.0))
            .collect(),
    )
    .unwrap()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

/// Three independent tasks with no dependencies should all be placed.
#[test]
fn hap_schedules_independent_tasks() {
    let h = horizon();

    let problem = make_problem(vec![block_from_task_ids(1, &[1, 2, 3])]);

    let mut possible_periods: TaskPeriodMap = HashMap::new();
    for id in 1..=3 {
        possible_periods.insert(TaskId(id), full_window(&h));
    }

    let scheduler = HapScheduler::default();
    let result = scheduler.run(&problem, &possible_periods, &h);
    assert!(result.is_ok(), "HAP must not error on a feasible problem");

    let schedule = result.unwrap();
    assert_eq!(schedule.len(), 3, "all 3 independent tasks must be placed");
}

/// A chain A→B→C: HAP must place all three and respect temporal ordering.
#[test]
fn hap_respects_dependency_ordering() {
    let h = horizon();
    let ids = [TaskId(10), TaskId(20), TaskId(30)];

    let mut possible_periods: TaskPeriodMap = HashMap::new();
    for &id in &ids {
        possible_periods.insert(id, full_window(&h));
    }

    let mut block = block_from_task_ids(1, &[10, 20, 30]);
    block
        .add_dependency(TaskId(10), TaskId(20), Dependency::DependsOn)
        .unwrap();
    block
        .add_dependency(TaskId(20), TaskId(30), Dependency::DependsOn)
        .unwrap();
    let problem = make_problem(vec![block]);

    let scheduler = HapScheduler::default();
    let schedule = scheduler
        .run(&problem, &possible_periods, &h)
        .expect("HAP must not error");

    assert_eq!(schedule.len(), 3, "all 3 tasks must be placed");

    let start = |id: TaskId| schedule.get(id).unwrap().start.value();
    assert!(
        start(TaskId(10)) < start(TaskId(20)),
        "A must start before B"
    );
    assert!(
        start(TaskId(20)) < start(TaskId(30)),
        "B must start before C"
    );
}

/// Running HAP twice with the same seed must produce identical schedules.
#[test]
fn hap_is_deterministic_with_fixed_seed() {
    let h = horizon();
    let problem = make_problem(vec![block_from_task_ids(1, &[1, 2, 3, 4, 5])]);

    let mut possible_periods: TaskPeriodMap = HashMap::new();
    for id in 1..=5 {
        possible_periods.insert(TaskId(id), full_window(&h));
    }

    let config = default_planner_config(); // seed = 0

    let run = |problem: &SchedulingProblem| -> Schedule {
        HapScheduler::new(config)
            .run(problem, &possible_periods, &h)
            .expect("HAP must not error")
    };

    let schedule_a = run(&problem);
    let schedule_b = run(&problem);

    // Same set of placed task IDs
    let placed_a: std::collections::HashSet<TaskId> =
        schedule_a.placements().map(|p| p.task_id).collect();
    let placed_b: std::collections::HashSet<TaskId> =
        schedule_b.placements().map(|p| p.task_id).collect();
    assert_eq!(placed_a, placed_b, "placed task sets must match");

    // Same start times for every placed task
    for id in placed_a {
        let sa = schedule_a.get(id).unwrap().start.value();
        let sb = schedule_b.get(id).unwrap().start.value();
        assert!(
            (sa - sb).abs() < 1e-12,
            "task {id:?} must start at same time in both runs"
        );
    }
}

/// A task with no feasible windows must not be placed — and HAP must not panic.
#[test]
fn hap_terminates_on_infeasible_problem() {
    let h = horizon();
    let problem = make_problem(vec![block_from_task_ids(1, &[1])]);

    // Empty PeriodSet → no feasible windows
    let mut possible_periods: TaskPeriodMap = HashMap::new();
    possible_periods.insert(TaskId(1), PeriodSet::new());

    let scheduler = HapScheduler::default();
    let result = scheduler.run(&problem, &possible_periods, &h);

    assert!(
        result.is_ok(),
        "HAP must return Ok even for infeasible tasks"
    );
    assert!(
        !result.unwrap().contains(TaskId(1)),
        "infeasible task must not be placed"
    );
}

/// With two blocks that share an overlapping window HAP must place at least one task.
#[test]
fn hap_completes_block_with_conflicts() {
    let h = horizon();

    // Both tasks want the first quarter of the horizon; they cannot both fit
    // in that slot but HAP should resolve the conflict and place at least one.
    let overlap_window = PeriodSet::from_periods(vec![Period::new(
        Time::<MJD>::new(60000.0),
        Time::<MJD>::new(60000.25),
    )]);

    let problem = make_problem(vec![
        block_from_task_ids(1, &[1]),
        block_from_task_ids(2, &[2]),
    ]);

    let mut possible_periods: TaskPeriodMap = HashMap::new();
    possible_periods.insert(TaskId(1), overlap_window.clone());
    possible_periods.insert(TaskId(2), overlap_window);

    let scheduler = HapScheduler::default();
    let result = scheduler.run(&problem, &possible_periods, &h);
    assert!(result.is_ok(), "HAP must not error");

    let schedule = result.unwrap();
    assert!(
        !schedule.is_empty(),
        "HAP must place at least one task when windows allow it"
    );
}

/// `default_planner_config()` must carry the documented HAP defaults.
#[test]
fn hap_default_config() {
    let cfg = default_planner_config();
    assert_eq!(cfg.population_size, 4);
    assert_eq!(cfg.cru.max_iter, 128);
    assert_eq!(cfg.cru.stochastic_range, 3);
    assert_eq!(cfg.seed, 0);
    assert!(matches!(cfg.cru.selector, Selector::Stochastic { rho: 3 }));
    assert!(matches!(
        cfg.survivor,
        SurvivorSelector::ElitistTopK { k: 4 }
    ));
    assert!(cfg.include_rejection_candidate);
}
