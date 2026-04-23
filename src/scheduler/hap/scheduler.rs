//! [`HapScheduler`] — the public entry point for the HAP algorithm.

use super::configuration::Configuration;
use super::cru::run_cru;
use super::proposal::Proposal;
use super::ranking::{compare_schedules, select_top_n, survivor_sets_equal};
use crate::error::ScheduleError;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::Schedule;
use crate::scheduling_block::SchedulingBlock;
use crate::task::Task;
use crate::time::{MJD, Period, SchedulingBlockId, TaskId};
use rayon::prelude::*;
use std::collections::HashMap;

/// HAP (Hybrid Asynchronous Proposal) scheduler.
///
/// Maintains a pool of `num_crus` survivor schedules and runs parallel CRU
/// workers to place one proposal's tasks at a time.  After each round the best
/// `num_crus` schedules are kept as survivors.  The algorithm terminates when
/// all proposals are complete or a full queue rotation produces no improvement.
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
    /// `tasks` must include every task referenced by any block in `blocks`.
    /// `possible_periods` must contain a feasibility window set for every task.
    pub fn run(
        &self,
        tasks: &HashMap<TaskId, Task>,
        possible_periods: &TaskPeriodMap,
        horizon: &Period<MJD>,
        blocks: &HashMap<SchedulingBlockId, SchedulingBlock>,
    ) -> Result<Schedule, ScheduleError> {
        // Guard against a degenerate num_crus=0 configuration.
        let num_crus = self.config.num_crus.max(1);

        log::info!(
            "hap: starting — blocks={}, tasks={}, num_crus={}, horizon=[{:.4}, {:.4}]",
            blocks.len(),
            tasks.len(),
            num_crus,
            horizon.start.value(),
            horizon.end.value(),
        );

        // Build task → block lookup
        let task_to_block: HashMap<TaskId, SchedulingBlockId> = blocks
            .iter()
            .flat_map(|(&block_id, block)| block.iter().map(move |task_id| (task_id, block_id)))
            .collect();

        // Build and sort proposals: highest priority first
        let mut proposals: Vec<Proposal> = blocks
            .values()
            .map(|block| Proposal::from_block(block, tasks, possible_periods, horizon.start))
            .collect();
        proposals.sort_by(|a, b| b.priority.total_cmp(&a.priority));

        if proposals.is_empty() {
            log::info!("hap: no proposals, returning empty schedule");
            return Ok(Schedule::new());
        }

        // Initialise survivor pool: num_crus copies of an empty schedule
        let empty = Schedule::new();
        let mut survivors: Vec<Schedule> = (0..num_crus).map(|_| empty.clone()).collect();

        let mut remaining: std::collections::VecDeque<Proposal> =
            proposals.iter().cloned().collect();
        let mut stall_counter: usize = 0;
        let mut round_index: u64 = 0;

        while !remaining.is_empty() {
            let queue_size = remaining.len();

            let proposal = remaining.pop_front().unwrap();

            log::debug!(
                "hap: round={} proposal={} priority={:.4} queue={}",
                round_index,
                proposal.id.0,
                proposal.priority,
                queue_size,
            );

            let prev_survivors = survivors.clone();

            // Assign survivor bases to CRUs (cycle when fewer survivors than workers)
            let cru_bases: Vec<Schedule> = (0..num_crus)
                .map(|i| survivors[i % survivors.len()].clone())
                .collect();

            let config = self.config;
            let proposals_ref = proposals.as_slice();

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
                        &proposal,
                        tasks,
                        possible_periods,
                        horizon,
                        blocks,
                        &task_to_block,
                        proposals_ref,
                        &config,
                        seed,
                    )
                })
                .collect();

            // Merge survivors + CRU outputs; keep best num_crus
            let mut all_candidates = survivors.clone();
            all_candidates.extend(cru_results);
            survivors = select_top_n(all_candidates, num_crus, &proposals);

            let proposal_complete = survivors.iter().any(|s| proposal.is_complete(s));

            if proposal_complete {
                log::debug!(
                    "hap: round={} proposal={} complete",
                    round_index,
                    proposal.id.0
                );
                stall_counter = 0;
            } else {
                // Re-queue at back for another attempt
                remaining.push_back(proposal);

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
            .max_by(|a, b| compare_schedules(b, a, &proposals)) // max = smallest in compare_schedules order
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
        let tasks: HashMap<TaskId, Task> = HashMap::new();
        let possible_periods = HashMap::new();
        let horizon = Period::new(
            crate::time::Time::<MJD>::new(60000.0),
            crate::time::Time::<MJD>::new(60001.0),
        );
        let blocks: HashMap<SchedulingBlockId, SchedulingBlock> = HashMap::new();

        let result = scheduler.run(&tasks, &possible_periods, &horizon, &blocks);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
