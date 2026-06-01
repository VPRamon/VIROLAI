//! Tests for the multi-cursor scheduler.
//!
//! The first two groups prove *exact* equivalence with the existing EST and LST
//! schedulers (single forward cursor == EST, single backward cursor == LST).
//! The remaining group exercises Plan A: multiple fixed-territory cursors that
//! share one schedule.

use super::MultiCursorScheduler;
use super::config::{CursorConfig, CursorPolicy, CursorTerritory, MultiCursorConfig};
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduler::est::{Configuration, EstScheduler};
use crate::scheduler::fom::{ScheduleFom, SoftConstraintFom};
use crate::scheduler::lst::LstScheduler;
use crate::scheduling_block::{Dependency, SchedulingBlock};
use crate::task::{IcrsTarget, Task};
use crate::time::{MJD, Period, SchedulingBlockId, TaskId, Time};
use qtty::{Degrees, Seconds};
use siderust::coordinates::frames::ICRS;
use siderust::coordinates::spherical::Direction;
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

fn task(id: u64, duration_days: f64, priority: f64) -> Task {
    use crate::constraints::{ConstraintExpr, PrioritySoftConstraint, SoftConstraintExpr};
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

/// Wrap independent tasks in singleton blocks.
fn problem_from_tasks(tasks: Vec<Task>) -> SchedulingProblem {
    let blocks = tasks
        .into_iter()
        .map(|t| SchedulingBlock::from_tasks(SchedulingBlockId(t.id.0), vec![t]))
        .collect::<Result<Vec<_>, _>>()
        .expect("blocks should be valid");
    SchedulingProblem::from_blocks(blocks).expect("problem should be valid")
}

fn fom() -> Arc<dyn ScheduleFom> {
    Arc::new(SoftConstraintFom)
}

/// Assert two schedules place exactly the same tasks at the same times.
fn assert_same_schedule(left: &Schedule, right: &Schedule) {
    assert_eq!(
        left.len(),
        right.len(),
        "schedules differ in placement count: {} vs {}",
        left.len(),
        right.len()
    );
    for placement in left.placements() {
        let other = right
            .get(placement.task_id)
            .unwrap_or_else(|| panic!("task {} missing from rhs schedule", placement.task_id.0));
        assert_eq!(
            placement.start, other.start,
            "task {} start differs",
            placement.task_id.0
        );
        assert_eq!(
            placement.end, other.end,
            "task {} end differs",
            placement.task_id.0
        );
    }
}

/// Assert no two placements in a schedule overlap.
fn assert_no_overlap(schedule: &Schedule) {
    let mut placements: Vec<_> = schedule.placements().collect();
    placements.sort_by(|a, b| a.start.value().total_cmp(&b.start.value()));
    for pair in placements.windows(2) {
        assert!(
            pair[0].end.value() <= pair[1].start.value() + 1e-9,
            "placements overlap: task {} [{}, {}) and task {} [{}, {})",
            pair[0].task_id.0,
            pair[0].start.value(),
            pair[0].end.value(),
            pair[1].task_id.0,
            pair[1].start.value(),
            pair[1].end.value(),
        );
    }
}

// --- Test scenarios reused across forward/backward equivalence ----------------

struct Scenario {
    task_specs: Vec<(u64, f64, f64)>,
    possible: TaskPeriodMap,
    horizon: Period<MJD>,
    config: Configuration,
}

impl Scenario {
    fn tasks(&self) -> Vec<Task> {
        self.task_specs
            .iter()
            .map(|&(id, dur, prio)| task(id, dur, prio))
            .collect()
    }
}

fn basic_scenario() -> Scenario {
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(2.0, 3.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 1.0)]));
    possible.insert(TaskId(3), windows(&[(4.0, 6.0)]));
    Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0), (3, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 8.0),
        config: Configuration::default(),
    }
}

fn endangered_scenario() -> Scenario {
    // Task 2 has a single tight window (endangered); task 1 is flexible and
    // would otherwise obstruct it.
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 6.0)]));
    possible.insert(TaskId(2), windows(&[(1.0, 2.0)]));
    Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 6.0),
        config: Configuration {
            k_beams: 1,
            branching_factor: 1,
            endangered_threshold: 2,
        },
    }
}

fn beam_scenario() -> Scenario {
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 3.0)]));
    possible.insert(TaskId(3), windows(&[(0.0, 4.0)]));
    Scenario {
        task_specs: vec![(1, 1.0, 10.0), (2, 1.0, 5.0), (3, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 5.0),
        config: Configuration {
            k_beams: 3,
            branching_factor: 2,
            endangered_threshold: 1,
        },
    }
}

fn soft_constraint_scenario() -> Scenario {
    // Two tasks share an identical window; FOM tie-breaking by priority decides.
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 4.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 4.0)]));
    possible.insert(TaskId(3), windows(&[(0.0, 4.0)]));
    Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 100.0), (3, 1.0, 50.0)],
        possible,
        horizon: period(0.0, 4.0),
        config: Configuration {
            k_beams: 2,
            branching_factor: 3,
            endangered_threshold: 1,
        },
    }
}

fn run_est(s: &Scenario) -> Schedule {
    let problem = problem_from_tasks(s.tasks());
    EstScheduler::from_parts(s.config, SoftConstraintFom)
        .expect("est config valid")
        .run(&problem, &s.possible, &s.horizon)
        .expect("est run")
}

fn run_lst(s: &Scenario) -> Schedule {
    let problem = problem_from_tasks(s.tasks());
    LstScheduler::from_parts(s.config, fom())
        .expect("lst config valid")
        .run(&problem, &s.possible, &s.horizon)
        .expect("lst run")
}

fn run_mc(config: MultiCursorConfig, s: &Scenario) -> Schedule {
    let problem = problem_from_tasks(s.tasks());
    MultiCursorScheduler::new(config, fom())
        .expect("mc config valid")
        .run(&problem, &s.possible, &s.horizon)
        .expect("mc run")
}

// --- M6: single forward cursor == EST ----------------------------------------

fn forward_config(s: &Scenario) -> MultiCursorConfig {
    MultiCursorConfig::single_forward(
        s.config.k_beams,
        s.config.branching_factor,
        s.config.endangered_threshold,
    )
}

#[test]
fn est_current_equals_multicursor_single_forward_basic() {
    let s = basic_scenario();
    assert_same_schedule(&run_est(&s), &run_mc(forward_config(&s), &s));
}

#[test]
fn est_current_equals_multicursor_single_forward_endangered() {
    let s = endangered_scenario();
    assert_same_schedule(&run_est(&s), &run_mc(forward_config(&s), &s));
}

#[test]
fn est_current_equals_multicursor_single_forward_beam() {
    let s = beam_scenario();
    assert_same_schedule(&run_est(&s), &run_mc(forward_config(&s), &s));
}

#[test]
fn est_current_equals_multicursor_single_forward_soft_constraints() {
    let s = soft_constraint_scenario();
    assert_same_schedule(&run_est(&s), &run_mc(forward_config(&s), &s));
}

#[test]
fn single_forward_constructor_matches_est() {
    let s = beam_scenario();
    let problem = problem_from_tasks(s.tasks());
    let mc = MultiCursorScheduler::single_forward(s.config, fom())
        .expect("constructor")
        .run(&problem, &s.possible, &s.horizon)
        .expect("run");
    assert_same_schedule(&run_est(&s), &mc);
}

// --- M7: single backward cursor == LST ---------------------------------------

fn backward_config(s: &Scenario) -> MultiCursorConfig {
    MultiCursorConfig::single_backward(
        s.config.k_beams,
        s.config.branching_factor,
        s.config.endangered_threshold,
    )
}

#[test]
fn lst_current_equals_multicursor_single_backward_basic() {
    let s = basic_scenario();
    assert_same_schedule(&run_lst(&s), &run_mc(backward_config(&s), &s));
}

#[test]
fn lst_current_equals_multicursor_single_backward_endangered() {
    let s = endangered_scenario();
    assert_same_schedule(&run_lst(&s), &run_mc(backward_config(&s), &s));
}

#[test]
fn lst_current_equals_multicursor_single_backward_beam() {
    let s = beam_scenario();
    assert_same_schedule(&run_lst(&s), &run_mc(backward_config(&s), &s));
}

#[test]
fn lst_current_equals_multicursor_single_backward_soft_constraints() {
    let s = soft_constraint_scenario();
    assert_same_schedule(&run_lst(&s), &run_mc(backward_config(&s), &s));
}

#[test]
fn single_backward_constructor_matches_lst() {
    let s = beam_scenario();
    let problem = problem_from_tasks(s.tasks());
    let mc = MultiCursorScheduler::single_backward(s.config, fom())
        .expect("constructor")
        .run(&problem, &s.possible, &s.horizon)
        .expect("run");
    assert_same_schedule(&run_lst(&s), &mc);
}

// --- M8: Plan A fixed territories --------------------------------------------

fn two_cursor_config(
    cursor0: CursorConfig,
    cursor1: CursorConfig,
    k_beams: usize,
    branching_factor: usize,
) -> MultiCursorConfig {
    MultiCursorConfig {
        cursors: vec![cursor0, cursor1],
        k_beams,
        branching_factor,
        endangered_threshold: 1,
        cursor_policy: CursorPolicy::BestCandidateGlobal,
    }
}

#[test]
fn multi_cursor_two_forward_fixed_territories_no_overlap() {
    // Front cursor owns [0, 5), back cursor owns [5, 10).
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), windows(&[(1.0, 4.0)]));
    possible.insert(TaskId(3), windows(&[(5.0, 7.0)]));
    possible.insert(TaskId(4), windows(&[(6.0, 9.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0), (3, 1.0, 1.0), (4, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 10.0),
        config: Configuration::default(),
    };

    let config = two_cursor_config(
        CursorConfig::forward(
            0,
            CursorTerritory::FractionRange {
                start: 0.0,
                end: 0.5,
            },
        ),
        CursorConfig::forward(
            1,
            CursorTerritory::FractionRange {
                start: 0.5,
                end: 1.0,
            },
        ),
        4,
        2,
    );
    let schedule = run_mc(config, &s);
    assert_no_overlap(&schedule);
    assert!(schedule.len() >= 2);
}

#[test]
fn multi_cursor_forward_backward_fixed_territories_no_overlap() {
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 4.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 4.0)]));
    possible.insert(TaskId(3), windows(&[(5.0, 9.0)]));
    possible.insert(TaskId(4), windows(&[(5.0, 9.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0), (3, 1.0, 1.0), (4, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 10.0),
        config: Configuration::default(),
    };

    let config = two_cursor_config(
        CursorConfig::forward(
            0,
            CursorTerritory::FractionRange {
                start: 0.0,
                end: 0.5,
            },
        ),
        CursorConfig::backward(
            1,
            CursorTerritory::FractionRange {
                start: 0.5,
                end: 1.0,
            },
        ),
        4,
        2,
    );
    let schedule = run_mc(config, &s);
    assert_no_overlap(&schedule);
}

#[test]
fn multi_cursor_forward_backward_both_cursors_contribute() {
    // Task 1 only fits in the first half, task 2 only in the second half.
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 2.0)]));
    possible.insert(TaskId(2), windows(&[(7.0, 9.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 10.0),
        config: Configuration::default(),
    };

    let config = two_cursor_config(
        CursorConfig::forward(
            0,
            CursorTerritory::FractionRange {
                start: 0.0,
                end: 0.5,
            },
        ),
        CursorConfig::backward(
            1,
            CursorTerritory::FractionRange {
                start: 0.5,
                end: 1.0,
            },
        ),
        4,
        2,
    );
    let schedule = run_mc(config, &s);
    assert_eq!(schedule.len(), 2, "both cursors should place their task");
    assert!(schedule.get(TaskId(1)).is_some());
    assert!(schedule.get(TaskId(2)).is_some());
    assert_no_overlap(&schedule);
}

#[test]
fn multi_cursor_rejects_cross_territory_placement() {
    // The only cursor owns [0, 3) but the task is only feasible in [4, 6):
    // it must remain unscheduled rather than escaping the territory.
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(4.0, 6.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 8.0),
        config: Configuration::default(),
    };

    let config = MultiCursorConfig {
        cursors: vec![CursorConfig::forward(
            0,
            CursorTerritory::Fixed {
                start: Time::<MJD>::new(0.0),
                end: Time::<MJD>::new(3.0),
            },
        )],
        k_beams: 1,
        branching_factor: 1,
        endangered_threshold: 1,
        cursor_policy: CursorPolicy::BestCandidateGlobal,
    };
    let schedule = run_mc(config, &s);
    assert_eq!(schedule.len(), 0, "task must not escape its territory");
}

#[test]
fn multi_cursor_does_not_duplicate_task_across_cursors() {
    // Both cursors can see task 1 (overlapping territories), but it must be
    // scheduled at most once.
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(2.0, 4.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 10.0),
        config: Configuration::default(),
    };

    let config = two_cursor_config(
        CursorConfig::forward(
            0,
            CursorTerritory::FractionRange {
                start: 0.0,
                end: 1.0,
            },
        ),
        CursorConfig::forward(
            1,
            CursorTerritory::FractionRange {
                start: 0.0,
                end: 1.0,
            },
        ),
        4,
        2,
    );
    let schedule = run_mc(config, &s);
    assert_eq!(
        schedule.len(),
        1,
        "task scheduled exactly once across cursors"
    );
}

#[test]
fn multi_cursor_respects_block_dependencies() {
    // Task 2 depends on task 1; the dependent must start no earlier than its
    // predecessor's end even across cursors.
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 3.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 6.0)]));

    let mut block = SchedulingBlock::from_tasks(
        SchedulingBlockId(1),
        vec![task(1, 1.0, 1.0), task(2, 1.0, 1.0)],
    )
    .expect("block valid");
    block
        .add_dependency(TaskId(1), TaskId(2), Dependency::DependsOn)
        .expect("dependency valid");
    let problem = SchedulingProblem::from_blocks(vec![block]).expect("problem valid");

    let possible_ref = &possible;
    let horizon = period(0.0, 6.0);
    let config = two_cursor_config(
        CursorConfig::forward(
            0,
            CursorTerritory::FractionRange {
                start: 0.0,
                end: 1.0,
            },
        ),
        CursorConfig::forward(
            1,
            CursorTerritory::FractionRange {
                start: 0.0,
                end: 1.0,
            },
        ),
        4,
        2,
    );
    let schedule = MultiCursorScheduler::new(config, fom())
        .expect("config valid")
        .run(&problem, possible_ref, &horizon)
        .expect("run");

    if let (Some(p1), Some(p2)) = (schedule.get(TaskId(1)), schedule.get(TaskId(2))) {
        assert!(
            p1.end.value() <= p2.start.value() + 1e-9,
            "dependency violated: predecessor ends after successor starts"
        );
    }
    assert_no_overlap(&schedule);
}

#[test]
fn dynamic_territory_is_unsupported() {
    use super::config::BoundaryRef;
    let territory = CursorTerritory::Dynamic {
        left: BoundaryRef::HorizonStart,
        right: BoundaryRef::HorizonEnd,
    };
    let err = territory.resolve(&period(0.0, 1.0)).unwrap_err();
    assert!(matches!(
        err,
        crate::error::ScheduleError::UnsupportedConfiguration(_)
    ));
}

#[test]
fn round_robin_policy_is_unsupported() {
    let config = MultiCursorConfig {
        cursors: vec![CursorConfig::forward(
            0,
            CursorTerritory::FractionRange {
                start: 0.0,
                end: 1.0,
            },
        )],
        k_beams: 1,
        branching_factor: 1,
        endangered_threshold: 1,
        cursor_policy: CursorPolicy::RoundRobin,
    };
    let err = MultiCursorScheduler::new(config, fom()).unwrap_err();
    assert!(matches!(
        err,
        crate::error::ScheduleError::UnsupportedConfiguration(_)
    ));
}
