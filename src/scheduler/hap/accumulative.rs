//! Accumulative-planner core shared by AP and HAP.
//!
//! AP and HAP differ only in [`PlannerConfig`]:
//!
//! | knob                          | AP                          | HAP                          |
//! |-------------------------------|-----------------------------|------------------------------|
//! | `cru.selector`                | [`Selector::Deterministic`] | [`Selector::Stochastic`]     |
//! | `population_size` (`ν`)       | 1                           | configured                   |
//! | `survivor`                    | `GreedyOne`                 | `ElitistTopK` / `ParetoFront`|
//! | output                        | one schedule                | set of schedules             |
//!
//! The control flow is identical: sort blocks by descending priority,
//! single pass, and per block: generate CRU candidates from one or more
//! source schedules, optionally include each source as a rejection
//! candidate, deduplicate, and reduce via the configured
//! [`SurvivorSelector`].
//!
//! [`Selector::Deterministic`]: super::configuration::Selector::Deterministic
//! [`Selector::Stochastic`]: super::configuration::Selector::Stochastic

use super::configuration::PlannerConfig;
use super::cru;
use super::eval::{block_priority, placement_fingerprint};
use super::selection;
use crate::prescheduler::TaskPeriodMap;
use crate::schedule::{Schedule, SchedulingProblem};
use crate::time::{MJD, Period};
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::collections::HashSet;

/// Run the accumulative planner against `problem` and return the surviving
/// schedules.
///
/// Returns at least one schedule. When the block list is empty the result
/// is `vec![input.clone()]`.
pub fn accumulative_plan(
    input: &Schedule,
    problem: &SchedulingProblem,
    periods: &TaskPeriodMap,
    horizon: &Period<MJD>,
    cfg: &PlannerConfig,
) -> Vec<Schedule> {
    let mut survivors: Vec<Schedule> = vec![input.clone()];

    let mut blocks: Vec<&_> = problem.blocks().iter().collect();
    if blocks.is_empty() {
        return survivors;
    }
    blocks.sort_by(|a, b| {
        block_priority(b, problem, horizon.start)
            .total_cmp(&block_priority(a, problem, horizon.start))
            .then_with(|| a.id.0.cmp(&b.id.0))
    });

    let population = cfg.population_size.max(1);
    let all_blocks = problem.blocks();

    for (block_idx, block) in blocks.into_iter().enumerate() {
        let mut candidates: Vec<Schedule> = Vec::new();
        let mut seen: HashSet<Vec<(u64, u64, u64)>> = HashSet::new();

        let mut maybe_push = |s: Schedule, candidates: &mut Vec<Schedule>| {
            let fp = placement_fingerprint(&s);
            if seen.insert(fp) {
                candidates.push(s);
            }
        };

        for source_idx in 0..population {
            let source = survivors[source_idx % survivors.len()].clone();

            let seed = cfg
                .seed
                .wrapping_add((block_idx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                .wrapping_add((source_idx as u64).wrapping_mul(0x6C62_272E_4BD9_4F09));
            let mut rng = StdRng::seed_from_u64(seed);

            let cru_results =
                cru::run_branches(&source, block, all_blocks, periods, &cfg.cru, &mut rng);

            if cfg.include_rejection_candidate {
                maybe_push(source, &mut candidates);
            }
            for s in cru_results {
                maybe_push(s, &mut candidates);
            }
        }

        if candidates.is_empty() {
            continue;
        }

        survivors = selection::select(cfg.survivor, candidates, problem, horizon.start);
    }

    if survivors.is_empty() {
        survivors.push(input.clone());
    }
    survivors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::{ConstraintBlocks, PrioritySoftConstraint, SoftConstraintExpr};
    use crate::scheduler::hap::configuration::{Selector, SurvivorSelector};
    use crate::scheduling_block::{CompletionExpr, SchedulingBlock};
    use crate::task::Task;
    use crate::time::{MJD, Period, PeriodSet, SchedulingBlockId, TaskId, Time};
    use qtty::Seconds;
    use siderust::coordinates::frames::ICRS;
    use siderust::coordinates::spherical::Direction;

    fn task_with_priority(id: u64, duration_days: f64, priority: f64) -> Task {
        let soft = if priority == 0.0 {
            None
        } else {
            Some(SoftConstraintExpr::atom(PrioritySoftConstraint::new(
                priority,
            )))
        };
        Task::new(
            TaskId(id),
            format!("t{id}"),
            Direction::<ICRS>::new_raw(0.0.into(), 0.0.into()),
            Seconds::new(duration_days * 86400.0),
            ConstraintBlocks::default(),
            soft,
        )
        .unwrap()
    }

    fn one_block(id: u64, task_id: u64, priority: f64) -> SchedulingBlock {
        let mut b = SchedulingBlock::from_tasks(
            SchedulingBlockId(id),
            vec![task_with_priority(task_id, 1.0, priority)],
        )
        .unwrap();
        b.set_completion(CompletionExpr::Leaf(TaskId(task_id)))
            .unwrap();
        b
    }

    fn windows(s: f64, e: f64) -> PeriodSet<MJD> {
        PeriodSet::from_periods(vec![Period::new(Time::<MJD>::new(s), Time::<MJD>::new(e))])
    }

    fn horizon() -> Period<MJD> {
        Period::new(Time::<MJD>::new(0.0), Time::<MJD>::new(100.0))
    }

    fn ap_cfg() -> PlannerConfig {
        PlannerConfig::ap(50)
    }

    /// Empty block list → returns `{input}`.
    #[test]
    fn empty_problem_returns_input() {
        let input = Schedule::new();
        let problem = SchedulingProblem::new();
        let periods = TaskPeriodMap::new();
        let out = accumulative_plan(&input, &problem, &periods, &horizon(), &ap_cfg());
        assert_eq!(out.len(), 1);
        assert!(out[0].is_empty());
    }

    /// AP places a feasible single-task block.
    #[test]
    fn ap_places_single_block() {
        let block = one_block(1, 10, 5.0);
        let mut problem = SchedulingProblem::new();
        problem.push_block(block).unwrap();

        let mut periods = TaskPeriodMap::new();
        periods.insert(TaskId(10), windows(0.0, 5.0));

        let out = accumulative_plan(&Schedule::new(), &problem, &periods, &horizon(), &ap_cfg());
        assert_eq!(out.len(), 1);
        assert!(out[0].contains(TaskId(10)));
    }

    /// AP processes the higher-priority block first (deterministic order).
    #[test]
    fn ap_sorts_blocks_by_priority_desc() {
        let low = one_block(1, 10, 1.0);
        let high = one_block(2, 20, 100.0);
        let mut problem = SchedulingProblem::new();
        problem.push_block(low).unwrap();
        problem.push_block(high).unwrap();

        let mut periods = TaskPeriodMap::new();
        periods.insert(TaskId(10), windows(0.0, 5.0));
        periods.insert(TaskId(20), windows(0.0, 5.0));

        let out = accumulative_plan(&Schedule::new(), &problem, &periods, &horizon(), &ap_cfg());
        assert_eq!(out.len(), 1);
        let placements: Vec<_> = out[0].placements().collect();
        // Higher-priority task should be placed at the start of its window.
        let high = placements.iter().find(|p| p.task_id == TaskId(20)).unwrap();
        assert_eq!(high.start, Time::<MJD>::new(0.0));
    }

    /// AP == HAP(ν=1, deterministic, GreedyOne).
    #[test]
    fn ap_equals_hap_pop1_deterministic() {
        let block = one_block(1, 10, 5.0);
        let mut problem = SchedulingProblem::new();
        problem.push_block(block).unwrap();

        let mut periods = TaskPeriodMap::new();
        periods.insert(TaskId(10), windows(0.0, 5.0));

        let ap = accumulative_plan(
            &Schedule::new(),
            &problem,
            &periods,
            &horizon(),
            &PlannerConfig::ap(50),
        );

        let hap_cfg = PlannerConfig {
            cru: super::super::configuration::Configuration {
                selector: Selector::Deterministic,
                max_iter: 50,
            },
            population_size: 1,
            survivor: SurvivorSelector::GreedyOne,
            include_rejection_candidate: true,
            seed: 0,
        };
        let hap = accumulative_plan(&Schedule::new(), &problem, &periods, &horizon(), &hap_cfg);

        assert_eq!(ap.len(), 1);
        assert_eq!(hap.len(), 1);
        assert_eq!(
            placement_fingerprint(&ap[0]),
            placement_fingerprint(&hap[0])
        );
    }

    /// HAP with ν>1, deterministic, GreedyOne should still collapse to one
    /// schedule (every source produces the same CRU output).
    #[test]
    fn hap_greedy_collapses_to_one() {
        let block = one_block(1, 10, 5.0);
        let mut problem = SchedulingProblem::new();
        problem.push_block(block).unwrap();

        let mut periods = TaskPeriodMap::new();
        periods.insert(TaskId(10), windows(0.0, 5.0));

        let hap_cfg = PlannerConfig {
            cru: super::super::configuration::Configuration {
                selector: Selector::Deterministic,
                max_iter: 50,
            },
            population_size: 4,
            survivor: SurvivorSelector::GreedyOne,
            include_rejection_candidate: true,
            seed: 42,
        };
        let out = accumulative_plan(&Schedule::new(), &problem, &periods, &horizon(), &hap_cfg);
        assert_eq!(out.len(), 1);
        assert!(out[0].contains(TaskId(10)));
    }

    /// HAP with Pareto and ν>1 returns at most `cap` schedules.
    #[test]
    fn hap_pareto_respects_cap() {
        let block = one_block(1, 10, 5.0);
        let mut problem = SchedulingProblem::new();
        problem.push_block(block).unwrap();

        let mut periods = TaskPeriodMap::new();
        periods.insert(TaskId(10), windows(0.0, 5.0));

        let hap_cfg = PlannerConfig::hap(50, 3, 4, SurvivorSelector::ParetoFront { cap: 2 }, 42);
        let out = accumulative_plan(&Schedule::new(), &problem, &periods, &horizon(), &hap_cfg);
        assert!(out.len() <= 2);
        assert!(!out.is_empty());
    }

    /// Rejection candidate keeps the previous survivor available even when
    /// CRU fails to place the block (no feasibility windows).
    #[test]
    fn rejection_candidate_keeps_survivor_when_cru_fails() {
        let block = one_block(1, 10, 5.0);
        let mut problem = SchedulingProblem::new();
        problem.push_block(block).unwrap();
        // No periods → CRU returns nothing.
        let periods = TaskPeriodMap::new();

        let out = accumulative_plan(&Schedule::new(), &problem, &periods, &horizon(), &ap_cfg());
        assert_eq!(out.len(), 1);
        assert!(out[0].is_empty());
    }
}
