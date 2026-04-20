use super::candidate::IntoTaskPlacement;
use super::fom::{ScheduleFom, TaskCountFom};
use super::queue::CandidateQueue;
use super::validation;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::task::Task;
use crate::time::{MJD, Period};
use std::sync::Arc;

/// Maximum allowed value for `k_beams`.
pub const MAX_K_BEAMS: usize = 100;

/// EST configuration — plain data, `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstConfig {
    /// Flexibility threshold below which a candidate is considered endangered.
    pub endangered_threshold: u32,
    /// Number of schedule states kept alive after each beam expansion round.
    pub k_beams: usize,
    /// Number of distinct candidates tried per beam per round.
    ///
    /// When `branching_factor == 1` and `k_beams == 1` the search degenerates
    /// to the classic greedy EST, matching the original single-beam behaviour
    /// exactly.
    pub branching_factor: usize,
}

impl Default for EstConfig {
    fn default() -> Self {
        Self {
            endangered_threshold: 1,
            k_beams: 1,
            branching_factor: 1,
        }
    }
}

/// EST scheduler implementation.
#[derive(Debug, Clone)]
pub struct EstScheduler {
    pub config: EstConfig,
    /// Figure of merit used to rank and prune beam states after each round.
    pub fom: Arc<dyn ScheduleFom>,
}

impl Default for EstScheduler {
    fn default() -> Self {
        Self {
            config: EstConfig::default(),
            fom: Arc::new(TaskCountFom),
        }
    }
}

impl EstScheduler {
    /// Create an `EstScheduler` with the given config and the default
    /// [`TaskCountFom`] figure of merit.
    pub fn new(config: EstConfig) -> Result<Self, ScheduleError> {
        let scheduler = Self {
            config,
            fom: Arc::new(TaskCountFom),
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
    pub fn run_scheduler(
        &self,
        tasks: Vec<Task>,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "est: starting scheduler — tasks={}, endangered_threshold={}, k_beams={}, branching_factor={}, horizon=[{:.4}, {:.4}]",
            tasks.len(),
            self.config.endangered_threshold,
            self.config.k_beams,
            self.config.branching_factor,
            horizon.start.value(),
            horizon.end.value(),
        );

        validation::validate_tasks(&tasks)?;
        let tasks = validation::filter_tasks(tasks, possible_periods);

        log::debug!("est: {} tasks remain after feasibility filter", tasks.len());

        let initial_candidates = CandidateQueue::build(
            &tasks,
            possible_periods,
            horizon,
            self.config.endangered_threshold,
        );

        let initial_state = super::ScheduleState {
            cursor: horizon.start,
            schedule: Schedule::new(),
            candidates: initial_candidates,
        };

        let mut live_beams: Vec<super::ScheduleState> = vec![initial_state];
        let mut terminal_beams: Vec<super::ScheduleState> = Vec::new();

        let k = self.config.k_beams;
        let b = self.config.branching_factor;
        let mut round: u32 = 0;

        while !live_beams.is_empty() {
            let mut next_scored: Vec<(f64, super::ScheduleState)> = Vec::new();

            for mut state in live_beams.drain(..) {
                state.candidates.refresh(
                    &Period::new(state.cursor, horizon.end),
                    self.config.endangered_threshold,
                );

                let n = state.candidates.count_schedulable();
                let branches = b.min(n);

                if branches == 0 {
                    terminal_beams.push(state);
                    continue;
                }

                for i in 0..branches {
                    let mut child = state.clone();
                    let candidate = child.candidates.pop_at(i);

                    let task_id = candidate.task_id();
                    let placement = candidate.into_task_placement(horizon.end);

                    log::debug!(
                        "est: round={} branch={}/{} placed task={} at [{:.4}, {:.4}]",
                        round,
                        i,
                        branches,
                        task_id.0,
                        placement.start.value(),
                        placement.end.value(),
                    );

                    child.cursor = placement.end.to::<MJD>();
                    child.schedule.insert_placement(placement);

                    let score = self.fom.evaluate(&child.schedule, &tasks);
                    next_scored.push((score, child));
                }
            }

            // Prune: keep top-k_beams states by FOM score.
            next_scored
                .sort_by(|(a, _), (b, _)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            next_scored.truncate(k);

            live_beams = next_scored.into_iter().map(|(_, s)| s).collect();
            round += 1;
        }

        let best = terminal_beams
            .into_iter()
            .max_by(|a, b| {
                let fa = self.fom.evaluate(&a.schedule, &tasks);
                let fb = self.fom.evaluate(&b.schedule, &tasks);
                fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("EST invariant violated: no terminal beam state produced");

        log::info!(
            "est: done — scheduled {} task(s) in {} round(s)",
            best.schedule.placements.len(),
            round,
        );

        Ok(best.schedule)
    }
}

pub fn run_scheduler(
    tasks: Vec<Task>,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError> {
    EstScheduler::default().run_scheduler(tasks, possible_periods, horizon)
}
