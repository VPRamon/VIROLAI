use super::ScheduleState;
use super::algorithm::EstScheduler;
use super::candidate::IntoTaskPlacement;
use super::context::{ProblemCtx, check_block_dependencies};
use crate::schedule::{Schedule, SchedulingProblem};
use crate::scheduler::fom::{FomContext, ScheduleFom};
use crate::time::{MJD, Period};
use std::cmp::Ordering;

enum BeamExpansion<'a> {
    Terminal(ScheduleState<'a>),
    Children(Vec<ScheduleState<'a>>),
}

/// Execute the EST beam-search loop starting from an already-built initial state.
///
/// This owns the branching, pruning, and terminal-state selection logic so the
/// outer algorithm module can focus on validation and initial queue setup.
pub(super) fn run_search<'a, F: ScheduleFom>(
    scheduler: &EstScheduler<F>,
    initial_state: ScheduleState<'a>,
    horizon: &Period<MJD>,
    problem: &SchedulingProblem,
    ctx: Option<&ProblemCtx<'_>>,
) -> Schedule {
    let mut live_beams: Vec<ScheduleState> = vec![initial_state];
    let mut terminal_beams: Vec<ScheduleState> = Vec::new();

    let k = scheduler.config.k_beams;
    let b = scheduler.config.branching_factor;
    let mut round: u32 = 0;

    while !live_beams.is_empty() {
        // Child beams generated in this round.
        // This is globally sorted and truncated so only the top-k states survive.
        let mut next_beams: Vec<ScheduleState<'a>> = Vec::new();

        for state in live_beams.drain(..) {
            match expand_beam(scheduler, state, horizon, round, b, problem, ctx) {
                BeamExpansion::Terminal(state) => terminal_beams.push(state),
                BeamExpansion::Children(children) => next_beams.extend(children),
            }
        }

        // Prune globally across all child beams produced this round. This is
        // where beam search diverges from the greedy single-path EST.
        next_beams.sort_by(|a, b| b.score.total_cmp(&a.score));
        next_beams.truncate(k); // Keep only the top-k schedules for the next round.

        live_beams = next_beams;
        round += 1;
    }

    // Return the best schedule among all terminal states using cached scores.
    let best_schedule = {
        let best = terminal_beams
            .into_iter()
            .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(Ordering::Equal))
            .expect("EST invariant violated: no terminal beam state produced");
        best.schedule
    };

    log::info!(
        "est: done — scheduled {} task(s) in {} round(s)",
        best_schedule.len(),
        round,
    );

    best_schedule
}

/// Expand one live beam into either terminal output or scored child beams.
///
/// This owns the per-beam scheduling logic so the outer search loop only
/// coordinates beam collection and pruning.
fn expand_beam<'a, F: ScheduleFom>(
    scheduler: &EstScheduler<F>,
    mut state: ScheduleState<'a>,
    horizon: &Period<MJD>,
    round: u32,
    branching_factor: usize,
    problem: &SchedulingProblem,
    ctx: Option<&ProblemCtx<'_>>,
) -> BeamExpansion<'a> {
    // Recompute EST metadata from the beam cursor to the end of the global
    // horizon before deciding what can branch next.
    state.candidates.refresh(
        &Period::new(state.cursor, horizon.end),
        scheduler.config.endangered_threshold,
    );

    let schedulable_count = state.candidates.count_schedulable();

    if schedulable_count == 0 {
        // This beam cannot place anything else, so it competes only at the
        // final terminal-state selection step.
        return BeamExpansion::Terminal(state);
    }

    let mut c0_placed = false;
    let mut children = Vec::new();
    for candidate_idx in 0..schedulable_count {
        // Dominance pruning: once c0 has been successfully placed, any
        // candidate whose raw EST is at or beyond c0's window end is
        // dominated — scheduling c0 first is always at least as good.
        // The guard `c0_placed` ensures we only skip when c0 was actually
        // accepted by domain validation; if c0 was rejected (e.g. an unmet
        // dependency), dominated candidates must still be explored.
        if c0_placed && state.candidates.is_dominated_by_first(candidate_idx) {
            log::trace!(
                "est: round={} candidate={} pruned (dominated by candidate 0)",
                round,
                candidate_idx,
            );
            continue;
        }
        if let Some(child) = build_child_state(
            scheduler,
            problem,
            &state,
            horizon,
            round,
            candidate_idx,
            schedulable_count,
            ctx,
        ) {
            c0_placed |= candidate_idx == 0;
            children.push(child);
            if children.len() == branching_factor {
                break;
            }
        }
    }

    // All branches may be pruned when domain validation rejects them (e.g.
    // every schedulable candidate has an unmet predecessor). In that case the
    // current state is as far as this beam can go.
    if children.is_empty() {
        return BeamExpansion::Terminal(state);
    }

    BeamExpansion::Children(children)
}

/// Build and score one child beam produced by choosing a single queue branch.
///
/// Returns `None` when domain validation rejects the placement (e.g. a
/// dependency predecessor has not been scheduled yet). The caller should treat
/// a `None` child as a pruned branch.
#[allow(clippy::too_many_arguments)]
fn build_child_state<'a, F: ScheduleFom>(
    scheduler: &EstScheduler<F>,
    problem: &SchedulingProblem,
    state: &ScheduleState<'a>,
    horizon: &Period<MJD>,
    round: u32,
    candidate_idx: usize,
    candidate_count: usize,
    ctx: Option<&ProblemCtx<'_>>,
) -> Option<ScheduleState<'a>> {
    let mut child = state.clone();
    // Candidate `candidate_idx` means "take the candidate_idx-th currently schedulable
    // candidate from the EST-ordered queue" and explore the schedule that
    // follows from that choice.
    let candidate = child.candidates.pop_at(candidate_idx);

    let task_id = candidate.task_id();
    let placement = candidate.into_task_placement(horizon.end);

    log::debug!(
        "est: round={} candidate={}/{} placed task={} at [{:.4}, {:.4}]",
        round,
        candidate_idx,
        candidate_count,
        task_id.0,
        placement.start.value(),
        placement.end.value(),
    );

    match ctx {
        Some(pctx) => {
            // Enforce intra-block dependency ordering.
            if let Err(err) =
                check_block_dependencies(&child.schedule, task_id, placement.start, pctx.problem)
            {
                log::debug!(
                    "est: round={} candidate={} task={} rejected by domain validation: {}",
                    round,
                    candidate_idx,
                    task_id.0,
                    err,
                );
                return None;
            }
            child.cursor = placement.end;
            child.schedule.insert_placement(placement);
        }
        None => {
            child.cursor = placement.end;
            child.schedule.insert_placement(placement);
        }
    }

    // FOM scoring is the pruning signal: higher-scoring child beams are more
    // likely to survive into the next round.
    let fom_ctx = FomContext {
        cursor: child.cursor,
        horizon: *horizon,
        possible_periods: ctx.map(|c| c.possible_periods),
    };
    child.score = scheduler.fom.evaluate(&child.schedule, problem, &fom_ctx);
    Some(child)
}
