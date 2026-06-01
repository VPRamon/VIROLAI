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
    // Since EstScheduler::run now delegates to the cursor engine, this helper
    // and run_mc(forward_config) exercise the exact same code path. The tests
    // that compare them serve as smoke tests verifying the wrapper delegation
    // is wired correctly.
    let problem = problem_from_tasks(s.tasks());
    EstScheduler::from_parts(s.config, SoftConstraintFom)
        .expect("est config valid")
        .run(&problem, &s.possible, &s.horizon)
        .expect("est run")
}

fn run_lst(s: &Scenario) -> Schedule {
    // Since LstScheduler::run now delegates to the cursor engine, this helper
    // and run_mc(backward_config) exercise the exact same code path. The tests
    // that compare them serve as smoke tests verifying the wrapper delegation
    // is wired correctly.
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

// --- M6: EST wrapper delegates to cursor engine (single-forward) --------------

fn forward_config(s: &Scenario) -> MultiCursorConfig {
    MultiCursorConfig::single_forward(
        s.config.k_beams,
        s.config.branching_factor,
        s.config.endangered_threshold,
    )
}

#[test]
fn est_wrapper_matches_multicursor_single_forward_basic() {
    let s = basic_scenario();
    assert_same_schedule(&run_est(&s), &run_mc(forward_config(&s), &s));
}

#[test]
fn est_wrapper_matches_multicursor_single_forward_endangered() {
    let s = endangered_scenario();
    assert_same_schedule(&run_est(&s), &run_mc(forward_config(&s), &s));
}

#[test]
fn est_wrapper_matches_multicursor_single_forward_beam() {
    let s = beam_scenario();
    assert_same_schedule(&run_est(&s), &run_mc(forward_config(&s), &s));
}

#[test]
fn est_wrapper_matches_multicursor_single_forward_soft_constraints() {
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

// --- M7: LST wrapper delegates to cursor engine (single-backward) -------------

fn backward_config(s: &Scenario) -> MultiCursorConfig {
    MultiCursorConfig::single_backward(
        s.config.k_beams,
        s.config.branching_factor,
        s.config.endangered_threshold,
    )
}

#[test]
fn lst_wrapper_matches_multicursor_single_backward_basic() {
    let s = basic_scenario();
    assert_same_schedule(&run_lst(&s), &run_mc(backward_config(&s), &s));
}

#[test]
fn lst_wrapper_matches_multicursor_single_backward_endangered() {
    let s = endangered_scenario();
    assert_same_schedule(&run_lst(&s), &run_mc(backward_config(&s), &s));
}

#[test]
fn lst_wrapper_matches_multicursor_single_backward_beam() {
    let s = beam_scenario();
    assert_same_schedule(&run_lst(&s), &run_mc(backward_config(&s), &s));
}

#[test]
fn lst_wrapper_matches_multicursor_single_backward_soft_constraints() {
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

// --- Plan B: dynamic territories --------------------------------------------

use super::config::{BoundaryRef, CursorDirection, CursorId};
use super::frame::CursorFrame;
use super::state::{CursorRuntime, CursorWorld};

/// Build a bare cursor runtime (empty queue) for boundary-resolution unit tests.
fn bare_cursor(
    id: usize,
    direction: CursorDirection,
    territory: CursorTerritory,
    extent: Period<MJD>,
    frame_cursor: Time<MJD>,
) -> CursorRuntime<'static> {
    let frame = match direction {
        CursorDirection::Forward => CursorFrame::Identity,
        CursorDirection::Backward => CursorFrame::Mirrored { territory: extent },
    };
    CursorRuntime {
        id: CursorId(id),
        direction,
        frame,
        territory,
        extent,
        frame_cursor,
        candidates: Vec::new(),
        exhausted: false,
    }
}

#[test]
fn dynamic_boundary_to_horizon_start_resolves() {
    let horizon = period(0.0, 10.0);
    let cursor = bare_cursor(
        0,
        CursorDirection::Forward,
        CursorTerritory::Dynamic {
            start: BoundaryRef::HorizonStart,
            end: BoundaryRef::HorizonEnd,
            min_gap: None,
        },
        horizon,
        Time::<MJD>::new(0.0),
    );
    let world = CursorWorld::snapshot(std::slice::from_ref(&cursor));
    let active = cursor
        .schedule_active_period(&world, &horizon)
        .expect("resolves")
        .expect("non-empty");
    assert_eq!(active.start.value(), 0.0);
    assert_eq!(active.end.value(), 10.0);
}

#[test]
fn dynamic_boundary_to_horizon_end_resolves() {
    let horizon = period(0.0, 10.0);
    // A backward cursor anchored at the horizon end: own position is the end.
    let cursor = bare_cursor(
        0,
        CursorDirection::Backward,
        CursorTerritory::Dynamic {
            start: BoundaryRef::HorizonStart,
            end: BoundaryRef::HorizonEnd,
            min_gap: None,
        },
        horizon,
        Time::<MJD>::new(0.0),
    );
    assert_eq!(cursor.schedule_position().value(), 10.0);
    let world = CursorWorld::snapshot(std::slice::from_ref(&cursor));
    let active = cursor
        .schedule_active_period(&world, &horizon)
        .expect("resolves")
        .expect("non-empty");
    assert_eq!(active.start.value(), 0.0);
    assert_eq!(active.end.value(), 10.0);
}

#[test]
fn dynamic_boundary_to_cursor_position_updates_after_placement() {
    let horizon = period(0.0, 10.0);
    let front = bare_cursor(
        0,
        CursorDirection::Forward,
        CursorTerritory::Dynamic {
            start: BoundaryRef::HorizonStart,
            end: BoundaryRef::Cursor(CursorId(1)),
            min_gap: None,
        },
        horizon,
        Time::<MJD>::new(0.0),
    );

    // Probe cursor 1 sitting at t = 7.
    let back_far = bare_cursor(
        1,
        CursorDirection::Forward,
        CursorTerritory::Fixed {
            start: Time::<MJD>::new(0.0),
            end: Time::<MJD>::new(10.0),
        },
        horizon,
        Time::<MJD>::new(7.0),
    );
    let world = CursorWorld::snapshot(&[front.clone(), back_far]);
    let active = front
        .schedule_active_period(&world, &horizon)
        .expect("resolves")
        .expect("non-empty");
    assert_eq!(active.end.value(), 7.0, "end follows cursor 1");

    // Cursor 1 advances to t = 4: the front cursor's end must shrink with it.
    let back_near = bare_cursor(
        1,
        CursorDirection::Forward,
        CursorTerritory::Fixed {
            start: Time::<MJD>::new(0.0),
            end: Time::<MJD>::new(10.0),
        },
        horizon,
        Time::<MJD>::new(4.0),
    );
    let world = CursorWorld::snapshot(&[front.clone(), back_near]);
    let active = front
        .schedule_active_period(&world, &horizon)
        .expect("resolves")
        .expect("non-empty");
    assert_eq!(active.end.value(), 4.0, "end tracks the moved cursor");
}

#[test]
fn dynamic_boundary_rejects_crossed_or_empty_active_period() {
    let horizon = period(0.0, 10.0);
    // Front cursor at t = 5, its end follows cursor 1 which is at t = 5: the
    // region [5, 5) is empty, so the cursor has no active period.
    let front = bare_cursor(
        0,
        CursorDirection::Forward,
        CursorTerritory::Dynamic {
            start: BoundaryRef::HorizonStart,
            end: BoundaryRef::Cursor(CursorId(1)),
            min_gap: None,
        },
        horizon,
        Time::<MJD>::new(5.0),
    );
    let back = bare_cursor(
        1,
        CursorDirection::Forward,
        CursorTerritory::Fixed {
            start: Time::<MJD>::new(0.0),
            end: Time::<MJD>::new(10.0),
        },
        horizon,
        Time::<MJD>::new(5.0),
    );
    let world = CursorWorld::snapshot(&[front.clone(), back]);
    assert!(
        front
            .schedule_active_period(&world, &horizon)
            .expect("resolves")
            .is_none(),
        "crossed cursors yield no active region"
    );
}

#[test]
fn dynamic_min_gap_keeps_cursors_apart() {
    let horizon = period(0.0, 10.0);
    let front = bare_cursor(
        0,
        CursorDirection::Forward,
        CursorTerritory::Dynamic {
            start: BoundaryRef::HorizonStart,
            end: BoundaryRef::Cursor(CursorId(1)),
            min_gap: Some(1.0),
        },
        horizon,
        Time::<MJD>::new(0.0),
    );
    let back = bare_cursor(
        1,
        CursorDirection::Forward,
        CursorTerritory::Fixed {
            start: Time::<MJD>::new(0.0),
            end: Time::<MJD>::new(10.0),
        },
        horizon,
        Time::<MJD>::new(6.0),
    );
    let world = CursorWorld::snapshot(&[front.clone(), back]);
    let active = front
        .schedule_active_period(&world, &horizon)
        .expect("resolves")
        .expect("non-empty");
    assert_eq!(active.end.value(), 5.0, "min_gap keeps a 1-day buffer");
}

#[test]
fn dynamic_config_rejects_self_reference() {
    let config = MultiCursorConfig {
        cursors: vec![CursorConfig::forward(
            0,
            CursorTerritory::Dynamic {
                start: BoundaryRef::HorizonStart,
                end: BoundaryRef::Cursor(CursorId(0)),
                min_gap: None,
            },
        )],
        k_beams: 1,
        branching_factor: 1,
        endangered_threshold: 1,
        cursor_policy: CursorPolicy::BestCandidateGlobal,
    };
    let err = MultiCursorScheduler::new(config, fom()).unwrap_err();
    assert!(matches!(
        err,
        crate::error::ScheduleError::InvalidConfiguration(_)
    ));
}

#[test]
fn dynamic_config_rejects_unknown_cursor_reference() {
    let config = MultiCursorConfig {
        cursors: vec![CursorConfig::forward(
            0,
            CursorTerritory::Dynamic {
                start: BoundaryRef::HorizonStart,
                end: BoundaryRef::Cursor(CursorId(9)),
                min_gap: None,
            },
        )],
        k_beams: 1,
        branching_factor: 1,
        endangered_threshold: 1,
        cursor_policy: CursorPolicy::BestCandidateGlobal,
    };
    let err = MultiCursorScheduler::new(config, fom()).unwrap_err();
    assert!(matches!(
        err,
        crate::error::ScheduleError::InvalidConfiguration(_)
    ));
}

// --- Plan B: dynamic EST+LST meet layout -------------------------------------

/// `n` unit tasks each feasible anywhere within `[0, n)`.
fn tiling_scenario(n: u64) -> Scenario {
    let mut possible = TaskPeriodMap::new();
    let mut specs = Vec::new();
    for id in 1..=n {
        possible.insert(TaskId(id), windows(&[(0.0, n as f64)]));
        specs.push((id, 1.0, 1.0));
    }
    Scenario {
        task_specs: specs,
        possible,
        horizon: period(0.0, n as f64),
        config: Configuration::default(),
    }
}

#[test]
fn dynamic_est_lst_cursors_move_until_meeting() {
    let s = tiling_scenario(6);
    let schedule = run_mc(MultiCursorConfig::dynamic_est_lst_meet(4, 2, 1), &s);
    assert_eq!(schedule.len(), 6, "two cursors tile the whole horizon");
    assert_no_overlap(&schedule);
}

#[test]
fn dynamic_est_lst_never_crosses() {
    let s = tiling_scenario(6);
    let schedule = run_mc(MultiCursorConfig::dynamic_est_lst_meet(4, 2, 1), &s);
    // Never crossing is observable as a valid, non-overlapping tiling: if the
    // cursors had crossed they would have double-booked or overlapped.
    assert_no_overlap(&schedule);
    assert_eq!(schedule.len(), 6);
}

#[test]
fn dynamic_est_lst_no_overlap() {
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 3.0)]));
    possible.insert(TaskId(2), windows(&[(2.0, 6.0)]));
    possible.insert(TaskId(3), windows(&[(4.0, 8.0)]));
    possible.insert(TaskId(4), windows(&[(7.0, 10.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0), (3, 1.0, 1.0), (4, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 10.0),
        config: Configuration::default(),
    };
    let schedule = run_mc(MultiCursorConfig::dynamic_est_lst_meet(4, 2, 1), &s);
    assert_no_overlap(&schedule);
}

#[test]
fn dynamic_est_lst_both_cursors_contribute() {
    // Task 1 only fits at the very start, task 2 only at the very end.
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 1.0)]));
    possible.insert(TaskId(2), windows(&[(9.0, 10.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 10.0),
        config: Configuration::default(),
    };
    let schedule = run_mc(MultiCursorConfig::dynamic_est_lst_meet(4, 2, 1), &s);
    assert_eq!(schedule.len(), 2);
    assert!(schedule.get(TaskId(1)).is_some());
    assert!(schedule.get(TaskId(2)).is_some());
    assert_no_overlap(&schedule);
}

#[test]
fn dynamic_est_lst_exhausted_cursor_does_not_stop_other_cursor() {
    // Every task is feasible only in the late region: the forward cursor's
    // active region collapses once the backward cursor advances past it, yet the
    // backward cursor must keep scheduling the remaining tasks.
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(7.0, 10.0)]));
    possible.insert(TaskId(2), windows(&[(7.0, 10.0)]));
    possible.insert(TaskId(3), windows(&[(7.0, 10.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0), (3, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 10.0),
        config: Configuration::default(),
    };
    let schedule = run_mc(MultiCursorConfig::dynamic_est_lst_meet(4, 2, 1), &s);
    assert_eq!(schedule.len(), 3, "all late tasks scheduled");
    assert_no_overlap(&schedule);
}

#[test]
fn dynamic_est_lst_does_not_duplicate_tasks() {
    let s = tiling_scenario(4);
    let schedule = run_mc(MultiCursorConfig::dynamic_est_lst_meet(4, 2, 1), &s);
    // Exactly four placements means none of the four tasks was double-booked.
    assert_eq!(schedule.len(), 4);
    assert_no_overlap(&schedule);
}

#[test]
fn dynamic_est_lst_respects_block_dependencies() {
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 6.0)]));
    possible.insert(TaskId(2), windows(&[(0.0, 10.0)]));

    let mut block = SchedulingBlock::from_tasks(
        SchedulingBlockId(1),
        vec![task(1, 1.0, 1.0), task(2, 1.0, 1.0)],
    )
    .expect("block valid");
    block
        .add_dependency(TaskId(1), TaskId(2), Dependency::DependsOn)
        .expect("dependency valid");
    let problem = SchedulingProblem::from_blocks(vec![block]).expect("problem valid");

    let horizon = period(0.0, 10.0);
    let schedule =
        MultiCursorScheduler::new(MultiCursorConfig::dynamic_est_lst_meet(4, 2, 1), fom())
            .expect("config valid")
            .run(&problem, &possible, &horizon)
            .expect("run");

    if let (Some(p1), Some(p2)) = (schedule.get(TaskId(1)), schedule.get(TaskId(2))) {
        assert!(
            p1.end.value() <= p2.start.value() + 1e-9,
            "dependency violated across cursors"
        );
    }
    assert_no_overlap(&schedule);
}

// --- Plan B: dynamic start+middle forward layout -----------------------------

#[test]
fn dynamic_start_mid_forward_front_respects_mid_cursor() {
    let s = tiling_scenario(6);
    let schedule = run_mc(MultiCursorConfig::dynamic_start_mid_forward(4, 2, 1), &s);
    // The front cursor may never invade the middle cursor's live region, so the
    // combined schedule stays overlap-free.
    assert_no_overlap(&schedule);
    assert!(schedule.len() >= 3);
}

#[test]
fn dynamic_start_mid_forward_mid_continues_to_horizon() {
    // Tasks only feasible in the far second half: only the middle cursor (which
    // owns [0.5, 1.0)) can reach them; the front cursor cannot.
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(7.0, 8.0)]));
    possible.insert(TaskId(2), windows(&[(8.0, 9.0)]));
    possible.insert(TaskId(3), windows(&[(9.0, 10.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0), (3, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 10.0),
        config: Configuration::default(),
    };
    let schedule = run_mc(MultiCursorConfig::dynamic_start_mid_forward(4, 2, 1), &s);
    assert_eq!(schedule.len(), 3, "middle cursor reaches the horizon end");
    assert_no_overlap(&schedule);
}

#[test]
fn dynamic_start_mid_forward_no_overlap() {
    let mut possible = TaskPeriodMap::new();
    possible.insert(TaskId(1), windows(&[(0.0, 4.0)]));
    possible.insert(TaskId(2), windows(&[(2.0, 7.0)]));
    possible.insert(TaskId(3), windows(&[(5.0, 10.0)]));
    let s = Scenario {
        task_specs: vec![(1, 1.0, 1.0), (2, 1.0, 1.0), (3, 1.0, 1.0)],
        possible,
        horizon: period(0.0, 10.0),
        config: Configuration::default(),
    };
    let schedule = run_mc(MultiCursorConfig::dynamic_start_mid_forward(4, 2, 1), &s);
    assert_no_overlap(&schedule);
}

#[test]
fn dynamic_start_mid_forward_does_not_duplicate_tasks() {
    let s = tiling_scenario(4);
    let schedule = run_mc(MultiCursorConfig::dynamic_start_mid_forward(4, 2, 1), &s);
    assert_no_overlap(&schedule);
    // No task may appear twice; the placement count never exceeds the task count.
    assert!(schedule.len() <= 4);
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
