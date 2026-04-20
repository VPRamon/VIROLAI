use super::candidate::IntoTaskPlacement;
use super::queue::CandidateQueue;
use super::validation;
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::task::Task;
use crate::time::{MJD, Period};

/// EST configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstConfig {
    /// Candidate class split between endangered and flexible.
    pub endangered_threshold: u32,
    /// Number of schedule states kept in memory during an EST run.
    pub k_beams: usize,
}

impl Default for EstConfig {
    fn default() -> Self {
        Self {
            endangered_threshold: 1,
            k_beams: 1,
        }
    }
}

/// EST scheduler implementation.
#[derive(Debug, Clone, Default)]
pub struct EstScheduler {
    pub config: EstConfig,
}

impl EstScheduler {
    pub fn new(config: EstConfig) -> Result<Self, ScheduleError> {
        let scheduler = Self { config };
        validation::validate_scheduler(&scheduler)?;
        Ok(scheduler)
    }

    /// Run EST on `task_ids` using the provided feasible windows.
    pub fn run_scheduler(
        &self,
        tasks: Vec<Task>,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        log::info!(
            "est: starting scheduler — tasks={}, endangered_threshold={}, k_beams={}, horizon=[{:.4}, {:.4}]",
            tasks.len(),
            self.config.endangered_threshold,
            self.config.k_beams,
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

        let mut schedule_states: Vec<super::ScheduleState> = (0..self.config.k_beams)
            .map(|_| super::ScheduleState {
                cursor: horizon.start,
                schedule: Schedule::new(),
                candidates: initial_candidates.clone(),
            })
            .collect();

        // while true
        //   for each schedule_state schedule next candidate
        //   if all schedule states have no candidates, break
        //   if collect k_beams schedules, keep best k_beams schedules and discard the rest


        let schedule_state = schedule_states
            .first_mut()
            .expect("EST invariant violated: k_beams must be >= 1");

        let mut iteration: u32 = 0;

        while schedule_state.cursor < horizon.end {
            schedule_state.candidates.refresh(
                &Period::new(schedule_state.cursor, horizon.end),
                self.config.endangered_threshold,
            );
            let Some(candidate) = schedule_state.candidates.pop_next() else {
                log::warn!(
                    "est: no schedulable candidates remain at cursor={:.4}",
                    schedule_state.cursor.value()
                );
                break;
            };

            let task_id = candidate.task_id();
            let placement = candidate.into_task_placement(horizon.end);

            log::debug!(
                "est: iteration={} placed task={} at [{:.4}, {:.4}]",
                iteration,
                task_id.0,
                placement.start.value(),
                placement.end.value(),
            );

            schedule_state.cursor = placement.end.to::<MJD>();
            schedule_state.schedule.insert_placement(placement);
            iteration += 1;
        }

        log::info!(
            "est: done — scheduled {} task(s) in {} iteration(s)",
            schedule_state.schedule.placements.len(),
            iteration,
        );

        let primary_state = schedule_states
            .into_iter()
            .next()
            .expect("EST invariant violated: primary schedule state is missing");

        Ok(primary_state.schedule)
    }
}

pub fn run_scheduler(
    tasks: Vec<Task>,
    possible_periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
) -> Result<Schedule, ScheduleError> {
    EstScheduler::default().run_scheduler(tasks, possible_periods, horizon)
}
