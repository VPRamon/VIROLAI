//! [`HapScheduler`] — the public entry point for the HAP algorithm.

use super::block_eval::{block_is_complete, block_priority};
use super::configuration::Configuration;
use super::cru::run_cru;
use super::ranking::{compare_schedules, select_top_n, survivor_sets_equal};
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::time::{MJD, Period, SchedulingBlockId};
use rayon::prelude::*;
use std::collections::VecDeque;

/// HAP scheduler.
///
/// Maintains a pool of `num_crus` survivor schedules and runs parallel CRU
/// workers to place one scheduling block's tasks at a time. After each round the best
/// `num_crus` schedules are kept as survivors.  The algorithm terminates when
/// all scheduling blocks are complete or a full queue rotation produces no
/// improvement.
#[derive(Default)]
pub struct HapScheduler {
    /// HAP tuning parameters.
    pub config: Configuration,
}

impl HapScheduler {
    /// Create a `HapScheduler` with the given configuration.
    pub fn new(config: Configuration) -> Self {
        Self { config }
    }

    /// Run the HAP algorithm.
    ///
    /// `possible_periods` must contain a feasibility window set for every task.
    pub fn run(
        &self,
        problem: &SchedulingProblem,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
    ) -> Result<Schedule, ScheduleError> {
        // Guard against a degenerate num_crus=0 configuration.
        let num_crus = self.config.num_crus.max(1);

        log::info!(
            "hap: starting — blocks={}, tasks={}, num_crus={}, horizon=[{:.4}, {:.4}]",
            problem.block_count(),
            problem.task_count(),
            num_crus,
            horizon.start.value(),
            horizon.end.value(),
        );

        let mut block_queue: VecDeque<SchedulingBlockId> =
            problem.blocks().iter().map(|block| block.id).collect();
        block_queue.make_contiguous().sort_by(|a, b| {
            let priority_a = problem
                .block(*a)
                .map(|block| block_priority(block, problem, horizon.start))
                .unwrap_or(0.0);
            let priority_b = problem
                .block(*b)
                .map(|block| block_priority(block, problem, horizon.start))
                .unwrap_or(0.0);
            priority_b.total_cmp(&priority_a)
        });

        if block_queue.is_empty() {
            log::info!("hap: no scheduling blocks, returning empty schedule");
            return Ok(Schedule::new());
        }

        // Initialise survivor pool: num_crus copies of an empty schedule
        let empty = Schedule::new();
        let mut survivors: Vec<Schedule> = (0..num_crus).map(|_| empty.clone()).collect();

        let mut stall_counter: usize = 0;
        let mut round_index: u64 = 0;

        while !block_queue.is_empty() {
            let queue_size = block_queue.len();
            let current_block_id = block_queue.pop_front().unwrap();
            let Some(current_block) = problem.block(current_block_id) else {
                log::warn!(
                    "hap: round={} block={} not found, skipping",
                    round_index,
                    current_block_id.0
                );
                continue;
            };
            let current_priority = block_priority(current_block, problem, horizon.start);

            log::debug!(
                "hap: round={} block={} priority={:.4} queue={}",
                round_index,
                current_block_id.0,
                current_priority,
                queue_size,
            );

            let prev_survivors = survivors.clone();

            // Assign survivor bases to CRUs (cycle when fewer survivors than workers)
            let cru_bases: Vec<Schedule> = (0..num_crus)
                .map(|i| survivors[i % survivors.len()].clone())
                .collect();

            let config = self.config;

            // Run CRUs in parallel
            let cru_results: Vec<Schedule> = cru_bases
                .into_par_iter()
                .enumerate()
                .map(|(cru_idx, base)| {
                    let seed = config
                        .random_seed
                        .wrapping_add(round_index.wrapping_mul(0x9E3779B9))
                        .wrapping_add(cru_idx as u64 * 0x6C62272E);
                    run_cru(
                        base,
                        current_block,
                        problem,
                        possible_periods,
                        horizon,
                        &config,
                        seed,
                    )
                })
                .collect();

            // Merge survivors + CRU outputs; keep best num_crus
            let mut all_candidates = survivors.clone();
            all_candidates.extend(cru_results);
            survivors = select_top_n(all_candidates, num_crus, problem, horizon.start);

            let block_complete = survivors
                .iter()
                .any(|schedule| block_is_complete(current_block, schedule));

            if block_complete {
                log::debug!(
                    "hap: round={} block={} complete",
                    round_index,
                    current_block_id.0
                );
                stall_counter = 0;
            } else {
                // Re-queue at back for another attempt
                block_queue.push_back(current_block_id);

                if survivor_sets_equal(&survivors, &prev_survivors) {
                    stall_counter += 1;
                    if stall_counter >= queue_size {
                        log::debug!(
                            "hap: terminating — full rotation stall after {} round(s)",
                            round_index
                        );
                        break;
                    }
                } else {
                    stall_counter = 0;
                }
            }

            round_index += 1;
        }

        // Return the best survivor
        let best = survivors
            .into_iter()
            .max_by(|a, b| compare_schedules(b, a, problem, horizon.start))
            .unwrap_or_default();

        log::info!(
            "hap: done — scheduled {} task(s) in {} round(s)",
            best.len(),
            round_index,
        );

        Ok(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn default_scheduler_has_expected_config() {
        let scheduler = HapScheduler::default();
        assert_eq!(scheduler.config.num_crus, 4);
        assert_eq!(scheduler.config.cru_max_iterations, 128);
        assert_eq!(scheduler.config.stochastic_range, 3);
    }

    #[test]
    fn new_scheduler_stores_config() {
        let config = Configuration {
            num_crus: 8,
            cru_max_iterations: 64,
            stochastic_range: 5,
            random_seed: 42,
            impatience_alpha: 2.0,
        };
        let scheduler = HapScheduler::new(config);
        assert_eq!(scheduler.config.num_crus, 8);
        assert_eq!(scheduler.config.random_seed, 42);
    }

    #[test]
    fn run_empty_blocks_returns_empty_schedule() {
        let scheduler = HapScheduler::default();
        let possible_periods = HashMap::new();
        let horizon = Period::new(
            crate::time::Time::<MJD>::new(60000.0),
            crate::time::Time::<MJD>::new(60001.0),
        );
        let problem = SchedulingProblem::new();

        let result = scheduler.run(&problem, &possible_periods, &horizon);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
