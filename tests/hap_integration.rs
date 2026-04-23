//! Integration tests for the HAP scheduler.

use std::collections::HashMap;

use qtty::Seconds;
use scheduler::{
    Period, PeriodSet, TaskPeriodMap,
    constraints::ConstraintExpr,
    schedule::Schedule,
    scheduler::hap::{Configuration, HapScheduler},
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

// ─── Tests ───────────────────────────────────────────────────────────────────

/// Three independent tasks with no dependencies should all be placed.
#[test]
fn hap_schedules_independent_tasks() {
    let h = horizon();

    let mut tasks = HashMap::new();
    for id in 1..=3 {
        let t = make_task(id, 600.0);
        tasks.insert(TaskId(id), t);
    }

    let mut possible_periods: TaskPeriodMap = HashMap::new();
    for id in 1..=3 {
        possible_periods.insert(TaskId(id), full_window(&h));
    }

    let mut block = SchedulingBlock::new(SchedulingBlockId(1));
    for id in 1..=3 {
        block.add_task(TaskId(id));
    }
    let mut blocks = HashMap::new();
    blocks.insert(SchedulingBlockId(1), block);

    let scheduler = HapScheduler::default();
    let result = scheduler.run(&tasks, &possible_periods, &h, &blocks);
    assert!(result.is_ok(), "HAP must not error on a feasible problem");

    let schedule = result.unwrap();
    assert_eq!(schedule.len(), 3, "all 3 independent tasks must be placed");
}

/// A chain A→B→C: HAP must place all three and respect temporal ordering.
#[test]
fn hap_respects_dependency_ordering() {
    let h = horizon();

    let ids = [TaskId(10), TaskId(20), TaskId(30)];
    let mut tasks = HashMap::new();
    for &id in &ids {
        tasks.insert(id, make_task(id.0, 600.0));
    }

    let mut possible_periods: TaskPeriodMap = HashMap::new();
    for &id in &ids {
        possible_periods.insert(id, full_window(&h));
    }

    let mut block = SchedulingBlock::new(SchedulingBlockId(1));
    block
        .add_dependency(TaskId(10), TaskId(20), Dependency::DependsOn)
        .unwrap();
    block
        .add_dependency(TaskId(20), TaskId(30), Dependency::DependsOn)
        .unwrap();
    let mut blocks = HashMap::new();
    blocks.insert(SchedulingBlockId(1), block);

    let scheduler = HapScheduler::default();
    let schedule = scheduler
        .run(&tasks, &possible_periods, &h, &blocks)
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

    let mut tasks = HashMap::new();
    for id in 1..=5 {
        tasks.insert(TaskId(id), make_task(id, 600.0));
    }

    let mut possible_periods: TaskPeriodMap = HashMap::new();
    for id in 1..=5 {
        possible_periods.insert(TaskId(id), full_window(&h));
    }

    let mut block = SchedulingBlock::new(SchedulingBlockId(1));
    for id in 1..=5 {
        block.add_task(TaskId(id));
    }
    let mut blocks = HashMap::new();
    blocks.insert(SchedulingBlockId(1), block);

    let config = Configuration::default(); // random_seed = 0

    let run = |blocks: &HashMap<SchedulingBlockId, SchedulingBlock>| -> Schedule {
        HapScheduler::new(config)
            .run(&tasks, &possible_periods, &h, blocks)
            .expect("HAP must not error")
    };

    let schedule_a = run(&blocks);
    let schedule_b = run(&blocks);

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

    let task = make_task(1, 600.0);
    let mut tasks = HashMap::new();
    tasks.insert(TaskId(1), task);

    // Empty PeriodSet → no feasible windows
    let mut possible_periods: TaskPeriodMap = HashMap::new();
    possible_periods.insert(TaskId(1), PeriodSet::new());

    let mut block = SchedulingBlock::new(SchedulingBlockId(1));
    block.add_task(TaskId(1));
    let mut blocks = HashMap::new();
    blocks.insert(SchedulingBlockId(1), block);

    let scheduler = HapScheduler::default();
    let result = scheduler.run(&tasks, &possible_periods, &h, &blocks);

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
fn hap_completes_proposal_with_conflicts() {
    let h = horizon();

    // Both tasks want the first quarter of the horizon; they cannot both fit
    // in that slot but HAP should resolve the conflict and place at least one.
    let overlap_window = PeriodSet::from_periods(vec![Period::new(
        Time::<MJD>::new(60000.0),
        Time::<MJD>::new(60000.25),
    )]);

    let task_a = make_task(1, 600.0);
    let task_b = make_task(2, 600.0);
    let mut tasks = HashMap::new();
    tasks.insert(TaskId(1), task_a);
    tasks.insert(TaskId(2), task_b);

    let mut possible_periods: TaskPeriodMap = HashMap::new();
    possible_periods.insert(TaskId(1), overlap_window.clone());
    possible_periods.insert(TaskId(2), overlap_window);

    let mut block_a = SchedulingBlock::new(SchedulingBlockId(1));
    block_a.add_task(TaskId(1));

    let mut block_b = SchedulingBlock::new(SchedulingBlockId(2));
    block_b.add_task(TaskId(2));

    let mut blocks = HashMap::new();
    blocks.insert(SchedulingBlockId(1), block_a);
    blocks.insert(SchedulingBlockId(2), block_b);

    let scheduler = HapScheduler::default();
    let result = scheduler.run(&tasks, &possible_periods, &h, &blocks);
    assert!(result.is_ok(), "HAP must not error");

    let schedule = result.unwrap();
    assert!(
        !schedule.is_empty(),
        "HAP must place at least one task when windows allow it"
    );
}

/// `Configuration::default()` must carry the documented default values.
#[test]
fn hap_default_config() {
    let cfg = Configuration::default();
    assert_eq!(cfg.num_crus, 4);
    assert_eq!(cfg.cru_max_iterations, 128);
    assert_eq!(cfg.stochastic_range, 3);
    assert_eq!(cfg.random_seed, 0);
    assert!((cfg.impatience_alpha - 1.0).abs() < f64::EPSILON);
}
