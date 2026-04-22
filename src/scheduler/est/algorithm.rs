use super::candidate::IntoTaskPlacement;
use super::config::EstConfig;
use super::fom::{EstFomKind, ScheduleFom, SoftConstraintFom};
use super::queue::CandidateQueue;
use super::validation;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{JD, MJD, Period, SchedulingBlockId, TaskId, Time};
use std::collections::HashMap;
use std::sync::Arc;

type ScoredState<'a> = (f64, super::ScheduleState<'a>);

enum BeamExpansion<'a> {
    Terminal(super::ScheduleState<'a>),
    Children(Vec<ScoredState<'a>>),
}

/// Domain context passed into the beam search when a [`crate::schedule::SchedulingProblem`]
/// is available.  Carrying it here allows the EST inner loop to validate
/// dependency ordering rather than bypassing domain invariants with a raw
/// `insert_placement`.
///
/// Hard-constraint coverage is intentionally not re-checked here: the
/// pre-scheduler already guarantees that every window in `possible_periods`
/// is constraint-feasible, and EST only proposes starts within those windows.
struct ProblemCtx<'p> {
    /// Pre-computed per-block task lists used for dependency checks.
    blocks: &'p HashMap<SchedulingBlockId, SchedulingBlock>,
}

/// EST scheduler implementation.
#[derive(Debug, Clone)]
pub struct EstScheduler {
    /// Search parameters controlling endangered detection, beam width, and branching.
    pub config: EstConfig,
    /// Figure of merit used to rank and prune beam states after each round.
    pub fom: Arc<dyn ScheduleFom>,
}

impl Default for EstScheduler {
    /// Construct the default single-beam EST scheduler scored by soft constraints.
    fn default() -> Self {
        Self {
            config: EstConfig::default(),
            fom: Arc::new(SoftConstraintFom),
        }
    }
}

impl EstScheduler {
    /// Create an `EstScheduler` with the given config and the default
    /// [`SoftConstraintFom`] figure of merit.
    pub fn new(config: EstConfig) -> Result<Self, ScheduleError> {
        let scheduler = Self {
            config,
            fom: Arc::new(SoftConstraintFom),
        };
        validation::validate_scheduler(&scheduler)?;
        Ok(scheduler)
    }

    /// Create an `EstScheduler` with a user-facing FOM kind selector.
    pub fn with_kind(config: EstConfig, fom_kind: EstFomKind) -> Result<Self, ScheduleError> {
        let scheduler = Self {
            config,
            fom: fom_kind.into_fom(),
        };
        validation::validate_scheduler(&scheduler)?;
        Ok(scheduler)
    }

    /// Create an `EstScheduler` with a custom figure of merit.
    pub fn with_fom(config: EstConfig, fom: Arc<dyn ScheduleFom>) -> Result<Self, ScheduleError> {
        let scheduler = Self { config, fom };
        validation::validate_scheduler(&scheduler)?;
        Ok(scheduler)
    }

    /// Run beam-search EST on `tasks` using the provided feasible windows.
    ///
    /// Each round every live beam is expanded by placing up to
    /// `branching_factor` distinct candidates. The resulting child states are
    /// evaluated with the configured FOM and the top `k_beams` survivors are
    /// carried into the next round. The best terminal state is returned.
    ///
    /// Decision flow per round:
    /// 1. Refresh each beam's queue against the current cursor.
    /// 2. Count currently schedulable candidates.
    /// 3. If none remain, the beam becomes terminal.
    /// 4. Otherwise branch on up to `branching_factor` queue entries.
    /// 5. Score all child beams with the configured FOM.
    /// 6. Keep only the top `k_beams` children globally.
    pub fn run_scheduler(
        &self,
        tasks: &[Task],
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "est: starting scheduler — tasks={}, k_beams={}, branching_factor={}, horizon=[{:.4}, {:.4}]",
            tasks.len(),
            self.config.k_beams,
            self.config.branching_factor,
            horizon.start.value(),
            horizon.end.value(),
        );

        validation::validate_tasks(tasks)?;
        let filtered_tasks = validation::filter_tasks(tasks, possible_periods);

        log::debug!(
            "est: {} tasks remain after feasibility filter",
            filtered_tasks.len()
        );

        let initial_candidates =
            CandidateQueue::build(&filtered_tasks, possible_periods, horizon, None);

        let initial_state = super::ScheduleState {
            cursor: horizon.start,
            schedule: Schedule::new(),
            candidates: initial_candidates,
        };

        Ok(self.run_search(tasks, initial_state, horizon, None))
    }

    /// Run beam-search EST through the domain model.
    ///
    /// Behaves like [`Self::run_scheduler`] but routes every placement through
    /// [`crate::schedule::SchedulingProblem::place_task`], which enforces
    /// dependency ordering in addition to the overlap and constraint checks
    /// already guaranteed by the prescheduler.
    ///
    /// Candidates that fail domain validation (e.g. a predecessor task has not
    /// yet been placed) are silently dropped from that beam branch rather than
    /// causing a hard error, because the prescheduler cannot pre-filter
    /// intra-block ordering violations.
    pub fn run_with_problem(
        &self,
        tasks: &[Task],
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
        blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "est: starting domain-aware scheduler — tasks={}, k_beams={}, branching_factor={}, horizon=[{:.4}, {:.4}]",
            tasks.len(),
            self.config.k_beams,
            self.config.branching_factor,
            horizon.start.value(),
            horizon.end.value(),
        );

        validation::validate_tasks(tasks)?;
        let filtered_tasks = validation::filter_tasks(tasks, possible_periods);

        log::debug!(
            "est: {} tasks remain after feasibility filter",
            filtered_tasks.len()
        );

        // Build task→block map so candidates carry their block affiliation.
        let task_block_map: HashMap<TaskId, SchedulingBlockId> = blocks
            .iter()
            .flat_map(|(&block_id, block)| block.iter().map(move |task_id| (task_id, block_id)))
            .collect();

        let ctx = ProblemCtx { blocks };

        let initial_candidates = CandidateQueue::build(
            &filtered_tasks,
            possible_periods,
            horizon,
            Some(&task_block_map),
        );

        let initial_state = super::ScheduleState {
            cursor: horizon.start,
            schedule: Schedule::new(),
            candidates: initial_candidates,
        };

        Ok(self.run_search(tasks, initial_state, horizon, Some(&ctx)))
    }

    /// Execute the EST beam-search loop starting from an already-built initial state.
    ///
    /// This helper owns the branching, pruning, and terminal-state selection
    /// logic so [`Self::run_scheduler`] can focus on validation and setup.
    fn run_search<'a>(
        &self,
        tasks: &[Task],
        initial_state: super::ScheduleState<'a>,
        horizon: &Period<MJD>,
        ctx: Option<&ProblemCtx<'_>>,
    ) -> Schedule {
        let mut live_beams: Vec<super::ScheduleState> = vec![initial_state];
        let mut terminal_beams: Vec<super::ScheduleState> = Vec::new();

        let k = self.config.k_beams;
        let b = self.config.branching_factor;
        let mut round: u32 = 0;

        while !live_beams.is_empty() {
            let mut next_scored: Vec<ScoredState<'a>> = Vec::new();

            for state in live_beams.drain(..) {
                match self.expand_beam(tasks, state, horizon, round, b, ctx) {
                    BeamExpansion::Terminal(state) => terminal_beams.push(state),
                    BeamExpansion::Children(children) => next_scored.extend(children),
                }
            }

            // Prune globally across all child beams produced this round. This is
            // where beam search diverges from the greedy single-path EST.
            next_scored.sort_by(|(a, _), (b, _)| b.total_cmp(a));
            next_scored.truncate(k);

            live_beams = next_scored.into_iter().map(|(_, s)| s).collect();
            round += 1;
        }

        let best = terminal_beams
            .into_iter()
            .max_by(|a, b| {
                let fa = self.fom.evaluate(&a.schedule, tasks);
                let fb = self.fom.evaluate(&b.schedule, tasks);
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
    /// This helper owns the per-beam scheduling logic so the outer search loop
    /// only coordinates beam collection and pruning.
    fn expand_beam<'a>(
        &self,
        tasks: &[Task],
        mut state: super::ScheduleState<'a>,
        horizon: &Period<MJD>,
        round: u32,
        branching_factor: usize,
        ctx: Option<&ProblemCtx<'_>>,
    ) -> BeamExpansion<'a> {
        // Recompute EST metadata from the beam cursor to the end of the
        // global horizon before deciding what can branch next.
        state
            .candidates
            .refresh(&Period::new(state.cursor, horizon.end));

        let schedulable = state.candidates.count_schedulable();
        let branches = branching_factor.min(schedulable);

        if branches == 0 {
            // This beam cannot place anything else, so it competes only
            // at the final terminal-state selection step.
            return BeamExpansion::Terminal(state);
        }

        let children: Vec<ScoredState<'a>> = (0..branches)
            .filter_map(|branch_idx| {
                self.build_child_state(tasks, &state, horizon, round, branch_idx, branches, ctx)
            })
            .collect();

        // All branches may be pruned when domain validation rejects them (e.g.
        // every schedulable candidate has an unmet predecessor).  In that case
        // the current state is as far as this beam can go.
        if children.is_empty() {
            return BeamExpansion::Terminal(state);
        }

        BeamExpansion::Children(children)
    }

    /// Build and score one child beam produced by choosing a single queue branch.
    ///
    /// Returns `None` when domain validation rejects the placement (e.g. a
    /// dependency predecessor has not been scheduled yet).  The caller should
    /// treat a `None` child as a pruned branch.
    #[allow(clippy::too_many_arguments)]
    fn build_child_state<'a>(
        &self,
        tasks: &[Task],
        state: &super::ScheduleState<'a>,
        horizon: &Period<MJD>,
        round: u32,
        branch_idx: usize,
        branch_count: usize,
        ctx: Option<&ProblemCtx<'_>>,
    ) -> Option<ScoredState<'a>> {
        let mut child = state.clone();
        // Branch `branch_idx` means "take the branch_idx-th currently
        // schedulable candidate from the EST-ordered queue" and explore the
        // schedule that follows from that choice.
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
                child.cursor = placement.end.to::<MJD>();
                child.schedule.insert_placement(placement);
            }
            None => {
                child.cursor = placement.end.to::<MJD>();
                child.schedule.insert_placement(placement);
            }
        }

        // FOM scoring is the pruning signal: higher-scoring child
        // beams are more likely to survive into the next round.
        let score = self.fom.evaluate(&child.schedule, tasks);
        Some((score, child))
    }
}

/// Convenience entry point for the default single-beam, task-count EST run.
pub fn run_scheduler(
    tasks: &[Task],
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError> {
    EstScheduler::default().run_scheduler(tasks, possible_periods, horizon)
}

/// Check that all predecessor tasks in the same block are already scheduled
/// and end before `candidate_start`.
///
/// Returns `Ok(())` if the placement is dependency-safe, or a
/// [`ScheduleError`] describing the violation.
fn check_block_dependencies(
    schedule: &Schedule,
    task_id: TaskId,
    candidate_start: Time<JD>,
    block_id: Option<SchedulingBlockId>,
    blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
) -> Result<(), ScheduleError> {
    let Some(block_id) = block_id else {
        return Ok(());
    };
    let Some(block) = blocks.get(&block_id) else {
        return Ok(());
    };
    if !block.contains_task(task_id) {
        return Ok(());
    }

    let order = block.topological_order()?;
    let task_pos = order.iter().position(|&t| t == task_id).unwrap_or(0);

    for &pred_id in order.iter().take(task_pos) {
        match schedule.get(pred_id) {
            None => {
                return Err(ScheduleError::ConstraintViolation(format!(
                    "task {} predecessor {} not yet scheduled",
                    task_id.0, pred_id.0,
                )));
            }
            Some(prev) if prev.end > candidate_start => {
                return Err(ScheduleError::ConstraintViolation(format!(
                    "task {} predecessor {} ends after candidate start",
                    task_id.0, pred_id.0,
                )));
            }
            Some(_) => {}
        }
    }

    Ok(())
}
