//! Beam-search engine for the multi-cursor scheduler.
//!
//! The engine generalises the EST beam search to several cursors that share one
//! global schedule. Each cursor runs *forward in its own frame* (see
//! [`super::frame`]); a forward cursor uses the identity frame and a backward
//! cursor a mirrored frame, so the engine itself never needs to know about
//! direction once the cursors are constructed.
//!
//! For a single forward cursor spanning the whole horizon the loop reduces
//! exactly to EST: same candidate ordering, same dominance pruning, same global
//! top-`k` pruning, same figure-of-merit context.
//!
//! # Territory resolution
//!
//! Before every expansion the engine snapshots each cursor's live frontier into
//! a [`CursorWorld`] and asks every cursor for its active region this round
//! (see [`CursorRuntime::schedule_active_period`]). Fixed (Plan A) and dynamic
//! (Plan B) territories share that one resolution path; the engine never
//! special-cases territory shape or cursor count.

use std::cmp::Ordering;

use super::action::{ActionRank, CursorAction};
use super::config::{CursorDirection, CursorPolicy, MultiCursorConfig};
use super::frame::CursorFrame;
use super::state::{CursorRuntime, CursorWorld, MultiCursorState, initial_cursor_time};
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem, TaskPlacement};
use crate::scheduler::est::{Candidate, IntoTaskPlacement, check_block_dependencies};
use crate::scheduler::filter_task_refs;
use crate::scheduler::fom::{CursorPosition, FomContext, ScheduleFom};
use crate::task::Task;
use crate::time::{MJD, Period};

/// One expanded beam: either a terminal state or its scored children.
enum BeamExpansion<'a> {
    Terminal(MultiCursorState<'a>),
    Children(Vec<MultiCursorState<'a>>),
}

/// Owned per-cursor data the candidate queues borrow from for the whole run.
struct CursorArena {
    /// Frame-time feasibility windows, one map per cursor (parallel to
    /// `MultiCursorConfig::cursors`).
    framed: Vec<TaskPeriodMap>,
    /// Maximal fixed extent (schedule time) per cursor — the frame axis.
    extent: Vec<Period<MJD>>,
    /// Frame per cursor.
    frame: Vec<CursorFrame>,
    /// Direction per cursor.
    direction: Vec<CursorDirection>,
}

/// Run the multi-cursor beam search.
///
/// `possible_periods` and `fom` are in the engine's schedule-time coordinate
/// space. Callers that need a globally mirrored space (the LST-equivalent path)
/// pre-mirror the periods/FOM and unmirror the result themselves.
pub(super) fn run_multi_cursor(
    config: &MultiCursorConfig,
    fom: &dyn ScheduleFom,
    problem: &SchedulingProblem,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError> {
    let filtered_tasks = filter_task_refs(problem.iter_tasks(), possible_periods);
    let arena = build_arena(config, possible_periods, horizon, &filtered_tasks)?;

    let initial = seed_state(config, &arena, &filtered_tasks, horizon);

    log::info!(
        "cursor: starting multi-cursor search — cursors={}, k_beams={}, branching_factor={}, endangered_threshold={}, fom={}",
        config.cursors.len(),
        config.k_beams,
        config.branching_factor,
        config.endangered_threshold,
        fom.label(),
    );

    let best = beam_loop(config, fom, problem, possible_periods, horizon, initial)?;
    Ok(best)
}

/// Build the owned per-cursor windows / frames.
fn build_arena(
    config: &MultiCursorConfig,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    filtered_tasks: &[&Task],
) -> Result<CursorArena, ScheduleError> {
    let mut framed = Vec::with_capacity(config.cursors.len());
    let mut extent = Vec::with_capacity(config.cursors.len());
    let mut frame = Vec::with_capacity(config.cursors.len());
    let mut direction = Vec::with_capacity(config.cursors.len());

    for cursor in &config.cursors {
        let region = cursor.territory.extent(horizon)?;
        let cursor_frame = match cursor.direction {
            CursorDirection::Forward => CursorFrame::Identity,
            CursorDirection::Backward => CursorFrame::Mirrored { territory: region },
        };

        let mut map = TaskPeriodMap::with_capacity(filtered_tasks.len());
        for task in filtered_tasks {
            let windows = possible_periods
                .get(&task.id)
                .expect("filtered task missing possible periods");
            map.insert(task.id, cursor_frame.to_frame_periods(windows));
        }

        framed.push(map);
        extent.push(region);
        frame.push(cursor_frame);
        direction.push(cursor.direction);
    }

    Ok(CursorArena {
        framed,
        extent,
        frame,
        direction,
    })
}

/// Build the initial beam state with one queue per cursor.
fn seed_state<'a>(
    config: &MultiCursorConfig,
    arena: &'a CursorArena,
    filtered_tasks: &[&'a Task],
    horizon: &Period<MJD>,
) -> MultiCursorState<'a> {
    let mut cursors = Vec::with_capacity(config.cursors.len());

    for (pos, cfg) in config.cursors.iter().enumerate() {
        let extent = arena.extent[pos];
        let frame = arena.frame[pos];
        // Seed the queue against the maximal frame window; the first expansion
        // refreshes every queue against its live active region.
        let initial_active = Period::new(extent.start, extent.end);

        let candidates: Vec<Candidate<'a>> = filtered_tasks
            .iter()
            .map(|task| {
                let windows = &arena.framed[pos][&task.id];
                Candidate::new(task, windows, &initial_active)
            })
            .collect();

        cursors.push(CursorRuntime {
            id: cfg.id,
            direction: arena.direction[pos],
            frame,
            territory: cfg.territory,
            extent,
            frame_cursor: initial_cursor_time(&extent),
            candidates,
            exhausted: false,
        });
    }

    MultiCursorState {
        schedule: Schedule::new(),
        cursors,
        score: 0.0,
        last_cursor_schedule_time: horizon.start,
        last_action_cursor: None,
    }
}

/// Drive the beam loop to completion and return the best schedule.
fn beam_loop<'a>(
    config: &MultiCursorConfig,
    fom: &dyn ScheduleFom,
    problem: &SchedulingProblem,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    initial: MultiCursorState<'a>,
) -> Result<Schedule, ScheduleError> {
    let mut live: Vec<MultiCursorState<'a>> = vec![initial];
    let mut terminal: Vec<MultiCursorState<'a>> = Vec::new();

    let k = config.k_beams;
    let b = config.branching_factor;
    let threshold = config.endangered_threshold;
    let mut round: u32 = 0;

    while !live.is_empty() {
        let mut next: Vec<MultiCursorState<'a>> = Vec::new();
        for state in live.drain(..) {
            match expand(
                config,
                fom,
                problem,
                possible_periods,
                horizon,
                threshold,
                b,
                round,
                state,
            )? {
                BeamExpansion::Terminal(state) => terminal.push(state),
                BeamExpansion::Children(children) => next.extend(children),
            }
        }

        next.sort_by(|a, b| b.score.total_cmp(&a.score));
        next.truncate(k);
        live = next;
        round += 1;
    }

    let best = terminal
        .into_iter()
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(Ordering::Equal))
        .expect("multi-cursor invariant violated: no terminal beam state produced");

    log::info!(
        "cursor: done — scheduled {} task(s) in {} round(s)",
        best.schedule.len(),
        round,
    );

    Ok(best.schedule)
}

/// Expand one live beam into a terminal state or scored children.
#[allow(clippy::too_many_arguments)]
fn expand<'a>(
    config: &MultiCursorConfig,
    fom: &dyn ScheduleFom,
    problem: &SchedulingProblem,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    threshold: u32,
    branching_factor: usize,
    round: u32,
    mut state: MultiCursorState<'a>,
) -> Result<BeamExpansion<'a>, ScheduleError> {
    // Snapshot every cursor's live frontier, then resolve and refresh each
    // queue against its active region for this round.
    let world = CursorWorld::snapshot(&state.cursors);
    let mut sched_actives: Vec<Option<Period<MJD>>> = Vec::with_capacity(state.cursors.len());
    for cursor in &state.cursors {
        sched_actives.push(cursor.schedule_active_period(&world, horizon)?);
    }
    for (pos, cursor) in state.cursors.iter_mut().enumerate() {
        let frame_active = sched_actives[pos].map(|p| cursor.frame.to_frame_period(&p));
        cursor.refresh(frame_active.as_ref(), threshold);
    }

    if state.total_schedulable() == 0 {
        return Ok(BeamExpansion::Terminal(state));
    }

    let actions = ranked_actions(config, &state);

    let mut c0_placed = vec![false; state.cursors.len()];
    let mut children = Vec::new();

    for action in actions {
        if c0_placed[action.cursor_pos]
            && state.cursors[action.cursor_pos].is_dominated_by_first(action.candidate_idx)
        {
            log::trace!(
                "cursor: round={} cursor={} candidate={} pruned (dominated)",
                round,
                action.cursor_id.0,
                action.candidate_idx,
            );
            continue;
        }

        if let Some(child) = build_child(
            fom,
            problem,
            possible_periods,
            horizon,
            &state,
            &sched_actives,
            action,
            round,
        ) {
            if action.candidate_idx == 0 {
                c0_placed[action.cursor_pos] = true;
            }
            children.push(child);
            if children.len() == branching_factor {
                break;
            }
        }
    }

    if children.is_empty() {
        return Ok(BeamExpansion::Terminal(state));
    }

    Ok(BeamExpansion::Children(children))
}

/// Collect and rank candidate actions across all cursors.
///
/// `BestCandidateGlobal` ranks by within-cursor rank, then cursor id, then task
/// id — a stable, deterministic total order.
fn ranked_actions(config: &MultiCursorConfig, state: &MultiCursorState<'_>) -> Vec<CursorAction> {
    debug_assert_eq!(config.cursor_policy, CursorPolicy::BestCandidateGlobal);

    let mut actions: Vec<CursorAction> = Vec::new();
    for (pos, cursor) in state.cursors.iter().enumerate() {
        let n = cursor.count_schedulable();
        for idx in 0..n {
            actions.push(CursorAction {
                cursor_pos: pos,
                cursor_id: cursor.id,
                candidate_idx: idx,
                rank: ActionRank(idx),
            });
        }
    }

    actions.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(a.cursor_id.cmp(&b.cursor_id))
            .then_with(|| {
                let ta = state.cursors[a.cursor_pos].task_at(a.candidate_idx);
                let tb = state.cursors[b.cursor_pos].task_at(b.candidate_idx);
                ta.map(|t| t.0).cmp(&tb.map(|t| t.0))
            })
    });

    actions
}

/// Build and score one child beam from a chosen action, or `None` when the
/// placement is rejected by domain validation.
#[allow(clippy::too_many_arguments)]
fn build_child<'a>(
    fom: &dyn ScheduleFom,
    problem: &SchedulingProblem,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    state: &MultiCursorState<'a>,
    sched_actives: &[Option<Period<MJD>>],
    action: CursorAction,
    round: u32,
) -> Option<MultiCursorState<'a>> {
    let mut child = state.clone();

    let pos = action.cursor_pos;
    let frame = child.cursors[pos].frame;
    let extent = child.cursors[pos].extent;

    // The live active region this cursor was refreshed against (pre-placement).
    // A schedulable candidate implies a non-empty region, but stay defensive.
    let active = sched_actives[pos]?;

    let candidate = child.cursors[pos].pop_at(action.candidate_idx);
    let task_id = candidate.task_id();

    let frame_placement = candidate.into_task_placement(extent.end);
    let frame_end = frame_placement.end;
    let placement = frame.to_schedule_placement(frame_placement);

    log::debug!(
        "cursor: round={} cursor={} candidate={} placed task={} at [{:.4}, {:.4}]",
        round,
        action.cursor_id.0,
        action.candidate_idx,
        task_id.0,
        placement.start.value(),
        placement.end.value(),
    );

    if let Err(err) = validate_multi_cursor_placement(&child.schedule, &placement, problem, &active)
    {
        log::debug!(
            "cursor: round={} cursor={} task={} rejected: {}",
            round,
            action.cursor_id.0,
            task_id.0,
            err,
        );
        return None;
    }

    child.cursors[pos].advance_to(frame_end);

    // Compute post-placement active periods: after the acting cursor has
    // advanced, re-snapshot all frontiers and resolve each cursor's active
    // region. Passing these to the FOM means direction-aware figures of merit
    // (e.g. FutureFlexibilityFom) receive the correct residual region:
    //   single-forward: [placement.end, horizon.end]   — legacy EST behaviour.
    //   single-backward (LST): [horizon.start, placement.start] — correct LST.
    let post_world = CursorWorld::snapshot(&child.cursors);
    let mut post_actives: Vec<Option<Period<MJD>>> = Vec::with_capacity(child.cursors.len());
    for cursor in &child.cursors {
        post_actives.push(
            cursor
                .schedule_active_period(&post_world, horizon)
                .ok()
                .flatten(),
        );
    }

    // A task scheduled by one cursor must become unavailable to all others.
    for (other_pos, other) in child.cursors.iter_mut().enumerate() {
        if other_pos != pos {
            other.remove_task(task_id);
        }
    }

    let schedule_cursor_time = placement.end;
    child.schedule.insert_placement(placement);
    child.last_cursor_schedule_time = schedule_cursor_time;
    child.last_action_cursor = Some(action.cursor_id.0);

    // Build the multi-cursor figure-of-merit context. `cursor` keeps the legacy
    // single-frontier signal (exact for single-forward EST / single-backward
    // LST); `cursor_positions` / `active_periods` / `last_action_cursor` expose
    // the full multi-cursor frontier for cursor-sensitive figures of merit.
    // None of these affect schedule validity, which is enforced entirely by
    // `validate_multi_cursor_placement`.
    let cursor_positions: Vec<CursorPosition> = child
        .cursors
        .iter()
        .map(|c| CursorPosition {
            id: c.id.0,
            position: c.schedule_position(),
        })
        .collect();

    let fom_ctx = FomContext {
        cursor: child.last_cursor_schedule_time,
        horizon: *horizon,
        possible_periods: Some(possible_periods),
        cursor_positions: Some(&cursor_positions),
        active_periods: Some(&post_actives),
        last_action_cursor: child.last_action_cursor,
    };
    child.score = fom.evaluate(&child.schedule, problem, &fom_ctx);

    Some(child)
}

/// Validate a placement against the shared schedule and the cursor's live
/// active region.
///
/// Checks, in order:
/// 1. the task is not already scheduled,
/// 2. block dependencies are satisfied,
/// 3. the placement lies within the cursor's active region this round,
/// 4. the placement does not overlap any existing placement.
pub(super) fn validate_multi_cursor_placement(
    schedule: &Schedule,
    placement: &TaskPlacement,
    problem: &SchedulingProblem,
    active: &Period<MJD>,
) -> Result<(), ScheduleError> {
    if schedule.contains(placement.task_id) {
        return Err(ScheduleError::ConstraintViolation(format!(
            "task {} already scheduled",
            placement.task_id.0
        )));
    }

    check_block_dependencies(schedule, placement.task_id, placement.start, problem)?;

    if placement.start.value() < active.start.value() || placement.end.value() > active.end.value()
    {
        return Err(ScheduleError::ConstraintViolation(format!(
            "task {} placement [{:.4}, {:.4}) escapes cursor active region [{:.4}, {:.4})",
            placement.task_id.0,
            placement.start.value(),
            placement.end.value(),
            active.start.value(),
            active.end.value(),
        )));
    }

    if !schedule.overlapping(&placement.interval()).is_empty() {
        return Err(ScheduleError::OverlapConflict);
    }

    Ok(())
}
