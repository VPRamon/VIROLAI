use super::ScheduleState;
use super::algorithm::EstScheduler;
use super::candidate::IntoTaskPlacement;
use super::context::{ProblemCtx, check_block_dependencies};
use crate::schedule::Schedule;
use crate::task::Task;
use crate::time::{MJD, Period};

/// Beam state paired with its FOM score for pruning.
type ScoredState<'a> = (f64, ScheduleState<'a>);

enum BeamExpansion<'a> {
    Terminal(ScheduleState<'a>),
    Children(Vec<ScoredState<'a>>),
}

/// Execute the EST beam-search loop starting from an already-built initial state.
///
/// This owns the branching, pruning, and terminal-state selection logic so the
/// outer algorithm module can focus on validation and initial queue setup.
pub(super) fn run_search<'a>(
    scheduler: &EstScheduler,
    tasks: &[Task],
    initial_state: ScheduleState<'a>,
    horizon: &Period<MJD>,
    ctx: Option<&ProblemCtx<'_>>,
) -> Schedule {
    let mut live_beams: Vec<ScheduleState> = vec![initial_state];
    let mut terminal_beams: Vec<ScheduleState> = Vec::new();

    let k = scheduler.config.k_beams;
    let b = scheduler.config.branching_factor;
    let mut round: u32 = 0;

    while !live_beams.is_empty() {
        // Child beams generated in this round, paired with their FOM score.
        // This is globally sorted and truncated so only the top-k states survive.
        let mut next_scored: Vec<ScoredState<'a>> = Vec::new();

        for state in live_beams.drain(..) {
            match expand_beam(scheduler, tasks, state, horizon, round, b, ctx) {
                BeamExpansion::Terminal(state) => terminal_beams.push(state),
                BeamExpansion::Children(children) => next_scored.extend(children),
            }
        }

        // Prune globally across all child beams produced this round. This is
        // where beam search diverges from the greedy single-path EST.
        next_scored.sort_by(|(a, _), (b, _)| b.total_cmp(a));
        next_scored.truncate(k); // Keep only the top-k schedules for the next round.

        live_beams = next_scored.into_iter().map(|(_, s)| s).collect();
        round += 1;
    }

    // Return the best schedule among all terminal states.
    let best = terminal_beams
        .into_iter()
        .max_by(|a, b| {
            let fa = scheduler.fom.evaluate(&a.schedule, tasks);
            let fb = scheduler.fom.evaluate(&b.schedule, tasks);
            fa.total_cmp(&fb)
        })
        .expect("EST invariant violated: no terminal beam state produced");

    log::info!(
        "est: done — scheduled {} task(s) in {} round(s)",
        best.schedule.len(),
        round,
    );

    best.schedule
}

/// Expand one live beam into either terminal output or scored child beams.
///
/// This owns the per-beam scheduling logic so the outer search loop only
/// coordinates beam collection and pruning.
fn expand_beam<'a>(
    scheduler: &EstScheduler,
    tasks: &[Task],
    mut state: ScheduleState<'a>,
    horizon: &Period<MJD>,
    round: u32,
    branching_factor: usize,
    ctx: Option<&ProblemCtx<'_>>,
) -> BeamExpansion<'a> {
    // Recompute EST metadata from the beam cursor to the end of the global
    // horizon before deciding what can branch next.
    state.candidates.refresh(
        &Period::new(state.cursor, horizon.end),
        scheduler.config.endangered_threshold,
    );

    let schedulable = state.candidates.count_schedulable();
    let branches = branching_factor.min(schedulable);

    if branches == 0 {
        // This beam cannot place anything else, so it competes only at the
        // final terminal-state selection step.
        return BeamExpansion::Terminal(state);
    }

    let children: Vec<ScoredState<'a>> = (0..branches)
        .filter_map(|branch_idx| {
            build_child_state(
                scheduler, tasks, &state, horizon, round, branch_idx, branches, ctx,
            )
        })
        .collect();

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
fn build_child_state<'a>(
    scheduler: &EstScheduler,
    tasks: &[Task],
    state: &ScheduleState<'a>,
    horizon: &Period<MJD>,
    round: u32,
    branch_idx: usize,
    branch_count: usize,
    ctx: Option<&ProblemCtx<'_>>,
) -> Option<ScoredState<'a>> {
    let mut child = state.clone();
    // Branch `branch_idx` means "take the branch_idx-th currently schedulable
    // candidate from the EST-ordered queue" and explore the schedule that
    // follows from that choice.
    let candidate = child.candidates.pop_at(branch_idx);

    let task_id = candidate.task_id();
    let placement = candidate.into_task_placement(horizon.end);

    log::debug!(
        "est: round={} branch={}/{} placed task={} at [{:.4}, {:.4}]",
        round,
        branch_idx,
        branch_count,
        task_id.0,
        placement.start.value(),
        placement.end.value(),
    );

    match ctx {
        Some(pctx) => {
            // Enforce intra-block dependency ordering.
            if let Err(err) = check_block_dependencies(
                &child.schedule,
                task_id,
                placement.start,
                placement.block_id,
                pctx.blocks,
            ) {
                log::debug!(
                    "est: round={} branch={} task={} rejected by domain validation: {}",
                    round,
                    branch_idx,
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
    let score = scheduler.fom.evaluate(&child.schedule, tasks);
    Some((score, child))
}
