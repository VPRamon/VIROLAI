//! Schedule evaluation metrics.
//!
//! [`ScheduleMetrics`] is the canonical, algorithm-agnostic measurement
//! surface used by the experiment-matrix runner and the webapp evaluation
//! environment. It is computed from a [`Schedule`] together with the source
//! [`SchedulingProblem`] and the scheduling horizon.
//!
//! The metric set is intentionally broader than the `est_experiment` legacy
//! `RunMetrics`: it includes completion ratios, full priority statistics,
//! schedule fragmentation (gap analysis), horizon utilization, per-resource
//! breakdown, and a configurable composite ranking score.
//!
//! Two design choices worth flagging:
//!
//! - `total_horizon_sec` and `available_time_sec` are derived from the
//!   horizon and the placements alone (no astronomical recomputation here),
//!   which keeps this module dependency-light and deterministic. Callers
//!   that already know an "available time" budget (e.g. computed from the
//!   prescheduler's feasible periods) can override it via
//!   [`MetricsContext::with_available_time_sec`] so fragmentation and
//!   utilization are normalized against the same baseline the scheduler
//!   itself saw.
//! - The current data model has at most one telescope/resource per problem,
//!   so per-resource breakdown is a single-entry vector. The shape is
//!   already plural so adding multi-resource support in the future does not
//!   require a metric-API break.

use crate::schedule::{Schedule, SchedulingProblem};
use crate::time::{MJD, Period, TaskId, Time};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Number of seconds in one mean solar day. Used to convert MJD-based
/// quantities into seconds.
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Optional inputs and overrides used when computing [`ScheduleMetrics`].
#[derive(Debug, Clone, Default)]
pub struct MetricsContext {
    /// Pre-computed available-time budget in seconds, used as the
    /// denominator for utilization and fragmentation. When `None`, the
    /// horizon length is used.
    pub available_time_sec: Option<f64>,
    /// Optional resource label to attach to the per-resource breakdown
    /// produced for the (single) telescope on the problem. Falls back to
    /// the telescope's own `name` and ultimately to `"resource-0"`.
    pub resource_label_override: Option<String>,
    /// Weights for the composite ranking score. `None` = equal weights for
    /// every available term.
    pub ranking: Option<RankingWeights>,
}

impl MetricsContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_available_time_sec(mut self, seconds: f64) -> Self {
        self.available_time_sec = Some(seconds);
        self
    }

    pub fn with_ranking(mut self, ranking: RankingWeights) -> Self {
        self.ranking = Some(ranking);
        self
    }
}

/// Weights for the composite ranking score.
///
/// The composite score is `Σ_t w_t · n_t` where `n_t ∈ [0, 1]` is the
/// normalized value of the metric `t`. Metrics where higher is better
/// (completion, priority, utilization) are normalized so that their best
/// observed value is `1.0`; fragmentation is inverted so that lower is
/// better. Because normalization is local to a single schedule, the
/// returned absolute score is mainly useful for cross-cell comparisons in
/// the experiment matrix (where the runner re-normalizes across cells).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RankingWeights {
    pub completion: f64,
    pub priority: f64,
    pub utilization: f64,
    pub fragmentation: f64,
}

impl Default for RankingWeights {
    fn default() -> Self {
        Self {
            completion: 1.0,
            priority: 1.0,
            utilization: 1.0,
            fragmentation: 1.0,
        }
    }
}

impl RankingWeights {
    /// Sum of all weights, used as the denominator when normalizing the
    /// composite score.
    pub fn total(&self) -> f64 {
        self.completion + self.priority + self.utilization + self.fragmentation
    }
}

/// Distributional summary of a numeric series.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PriorityStats {
    pub count: usize,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub std: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p90: f64,
}

/// Idle-gap analysis over a sorted, non-overlapping set of placements.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct FragmentationStats {
    /// Number of idle gaps strictly between consecutive placements.
    pub gap_count: usize,
    /// Total idle time inside the placement envelope, in seconds.
    pub gap_total_sec: f64,
    /// Largest single idle gap in seconds.
    pub largest_gap_sec: f64,
    /// `gap_total_sec / available_time_sec`. `0.0` when no schedule is
    /// possible. Higher means more fragmented.
    pub fragmentation_index: f64,
}

/// Per-resource scheduling breakdown. Currently the data model exposes one
/// telescope per problem, but the type is a `Vec<ResourceMetrics>` so the
/// metric surface does not need to change when multi-resource support is
/// introduced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceMetrics {
    pub resource_id: String,
    pub scheduled_task_count: usize,
    pub scheduled_time_sec: f64,
    pub priority_sum: f64,
    pub utilization: f64,
}

/// The full metric surface for one scheduler run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleMetrics {
    pub scheduled_task_count: usize,
    pub total_task_count: usize,
    pub completion_ratio: f64,
    pub priority: PriorityStats,
    pub fragmentation: FragmentationStats,
    pub total_horizon_sec: f64,
    pub available_time_sec: f64,
    pub scheduled_time_sec: f64,
    pub utilization: f64,
    pub per_resource: Vec<ResourceMetrics>,
    pub composite_rank_score: f64,
    pub ranking_weights: RankingWeights,
}

impl ScheduleMetrics {
    /// Compute the full metric surface for `schedule` against `problem` over
    /// `horizon`, applying any overrides in `context`.
    pub fn compute(
        schedule: &Schedule,
        problem: &SchedulingProblem,
        horizon: &Period<MJD>,
        context: &MetricsContext,
    ) -> Self {
        let priority_by_task = collect_task_priorities(problem, horizon.start);
        let scheduled_priorities: Vec<f64> = schedule
            .placements()
            .map(|p| *priority_by_task.get(&p.task_id).unwrap_or(&0.0))
            .collect();

        let scheduled_task_count = schedule.len();
        let total_task_count = problem.task_count();
        let completion_ratio = if total_task_count == 0 {
            0.0
        } else {
            scheduled_task_count as f64 / total_task_count as f64
        };

        let total_horizon_sec = (horizon.end.value() - horizon.start.value()) * SECONDS_PER_DAY;
        let available_time_sec = context.available_time_sec.unwrap_or(total_horizon_sec);

        let scheduled_time_sec: f64 = schedule
            .placements()
            .map(|p| (p.end.value() - p.start.value()) * SECONDS_PER_DAY)
            .sum();

        let utilization = ratio(scheduled_time_sec, available_time_sec);

        let priority = PriorityStats::from_values(&scheduled_priorities);
        let fragmentation = FragmentationStats::from_schedule(schedule, available_time_sec);

        let resource_label = context
            .resource_label_override
            .clone()
            .or_else(|| problem.telescope.as_ref().map(|t| t.name.clone()))
            .unwrap_or_else(|| "resource-0".to_string());
        let per_resource = vec![ResourceMetrics {
            resource_id: resource_label,
            scheduled_task_count,
            scheduled_time_sec,
            priority_sum: priority.sum,
            utilization,
        }];

        let ranking_weights = context.ranking.unwrap_or_default();
        let composite_rank_score = composite_score(
            completion_ratio,
            &priority,
            utilization,
            &fragmentation,
            ranking_weights,
        );

        Self {
            scheduled_task_count,
            total_task_count,
            completion_ratio,
            priority,
            fragmentation,
            total_horizon_sec,
            available_time_sec,
            scheduled_time_sec,
            utilization,
            per_resource,
            composite_rank_score,
            ranking_weights,
        }
    }
}

impl PriorityStats {
    /// Compute summary statistics over `values`. Returns the zero default
    /// when the slice is empty.
    pub fn from_values(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }

        let count = values.len();
        let sum: f64 = values.iter().copied().sum();
        let mean = sum / count as f64;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut sq_dev_sum = 0.0;
        for &v in values {
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
            let d = v - mean;
            sq_dev_sum += d * d;
        }
        let std = if count > 1 {
            (sq_dev_sum / count as f64).sqrt()
        } else {
            0.0
        };

        Self {
            count,
            sum,
            min,
            max,
            mean,
            std,
            p25: percentile(values, 0.25),
            p50: percentile(values, 0.50),
            p75: percentile(values, 0.75),
            p90: percentile(values, 0.90),
        }
    }
}

impl FragmentationStats {
    /// Compute idle-gap statistics for `schedule`, normalized against
    /// `available_time_sec`.
    pub fn from_schedule(schedule: &Schedule, available_time_sec: f64) -> Self {
        let mut intervals: Vec<(f64, f64)> = schedule
            .placements()
            .map(|p| (p.start.value(), p.end.value()))
            .collect();
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut gap_count = 0usize;
        let mut gap_total_days = 0.0;
        let mut largest_gap_days = 0.0;
        for window in intervals.windows(2) {
            let prev_end = window[0].1;
            let next_start = window[1].0;
            if next_start > prev_end {
                let gap = next_start - prev_end;
                gap_count += 1;
                gap_total_days += gap;
                if gap > largest_gap_days {
                    largest_gap_days = gap;
                }
            }
        }

        let gap_total_sec = gap_total_days * SECONDS_PER_DAY;
        let largest_gap_sec = largest_gap_days * SECONDS_PER_DAY;
        let fragmentation_index = ratio(gap_total_sec, available_time_sec);

        Self {
            gap_count,
            gap_total_sec,
            largest_gap_sec,
            fragmentation_index,
        }
    }
}

fn collect_task_priorities(
    problem: &SchedulingProblem,
    horizon_start: Time<MJD>,
) -> HashMap<TaskId, f64> {
    problem
        .iter_tasks()
        .map(|task| {
            let value = task
                .soft_constraints
                .as_ref()
                .map(|expr| expr.score(&horizon_start, None, Some(&task.target)))
                .unwrap_or(0.0);
            (task.id, value)
        })
        .collect()
}

fn composite_score(
    completion_ratio: f64,
    priority: &PriorityStats,
    utilization: f64,
    fragmentation: &FragmentationStats,
    weights: RankingWeights,
) -> f64 {
    let total_weight = weights.total();
    if total_weight <= 0.0 {
        return 0.0;
    }

    // For "higher is better" terms, completion and utilization are already
    // in [0, 1]. Priority and fragmentation are normalized into [0, 1]
    // locally:
    //   - priority: divided by `count * max` to land in [0, 1]
    //   - fragmentation: 1 - clamp(fragmentation_index, 0.0, 1.0)
    let priority_term = if priority.count == 0 || priority.max <= 0.0 {
        0.0
    } else {
        let denom = priority.count as f64 * priority.max;
        if denom <= 0.0 {
            0.0
        } else {
            (priority.sum / denom).clamp(0.0, 1.0)
        }
    };
    let fragmentation_term = (1.0 - fragmentation.fragmentation_index.clamp(0.0, 1.0)).max(0.0);
    let utilization_term = utilization.clamp(0.0, 1.0);
    let completion_term = completion_ratio.clamp(0.0, 1.0);

    let weighted = weights.completion * completion_term
        + weights.priority * priority_term
        + weights.utilization * utilization_term
        + weights.fragmentation * fragmentation_term;

    weighted / total_weight
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator <= 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

/// Linear-interpolation percentile over `[0.0, 1.0]`. Returns `0.0` for an
/// empty slice. Matches the percentile defined in `est_experiment` so the
/// new metric module is a strict superset of the legacy one.
fn percentile(values: &[f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    if values.len() == 1 {
        return values[0];
    }
    let quantile = quantile.clamp(0.0, 1.0);
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = quantile * (sorted.len() as f64 - 1.0);
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let fraction = rank - lower as f64;
        sorted[lower] + fraction * (sorted[upper] - sorted[lower])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::{
        ConstraintBlocks, ConstraintExpr, PrioritySoftConstraint, SoftConstraintExpr,
    };
    use crate::schedule::TaskPlacement;
    use crate::scheduling_block::SchedulingBlock;
    use crate::scheduling_block::task::Task;
    use crate::time::{MJD, Period, SchedulingBlockId, TaskId, Time};
    use qtty::{Degrees, Seconds};
    use siderust::coordinates::frames::ICRS;
    use siderust::coordinates::spherical::Direction;

    fn priority_constraint(value: f64) -> Option<SoftConstraintExpr> {
        Some(SoftConstraintExpr::atom(PrioritySoftConstraint::new(value)))
    }

    fn build_task(id: u64, duration_sec: f64, priority: Option<f64>) -> Task {
        Task::new(
            TaskId(id),
            format!("task-{id}"),
            Direction::<ICRS>::new_raw(Degrees::new(10.0), Degrees::new(20.0)),
            Seconds::new(duration_sec),
            ConstraintBlocks::from(ConstraintExpr::Intersection(vec![])),
            priority.and_then(priority_constraint),
        )
        .unwrap()
    }

    fn problem_with_priorities(priorities: &[(u64, f64)]) -> SchedulingProblem {
        let tasks: Vec<Task> = priorities
            .iter()
            .map(|&(id, p)| build_task(id, 600.0, Some(p)))
            .collect();
        let block = SchedulingBlock::from_tasks(SchedulingBlockId(1), tasks).unwrap();
        SchedulingProblem::from_blocks(vec![block]).unwrap()
    }

    fn empty_problem() -> SchedulingProblem {
        SchedulingProblem::default()
    }

    fn place(schedule: &mut Schedule, task_id: u64, start: f64, end: f64) {
        schedule.insert_placement(TaskPlacement {
            task_id: TaskId(task_id),
            start: Time::<MJD>::new(start),
            end: Time::<MJD>::new(end),
        });
    }

    fn horizon(start: f64, end: f64) -> Period<MJD> {
        Period::new(Time::<MJD>::new(start), Time::<MJD>::new(end))
    }

    #[test]
    fn empty_schedule_returns_zeroed_metrics() {
        let problem = empty_problem();
        let schedule = Schedule::new();
        let horizon = horizon(0.0, 1.0);
        let metrics =
            ScheduleMetrics::compute(&schedule, &problem, &horizon, &MetricsContext::default());

        assert_eq!(metrics.scheduled_task_count, 0);
        assert_eq!(metrics.total_task_count, 0);
        assert_eq!(metrics.completion_ratio, 0.0);
        assert_eq!(metrics.priority, PriorityStats::default());
        assert_eq!(metrics.fragmentation, FragmentationStats::default());
        assert_eq!(metrics.scheduled_time_sec, 0.0);
        assert_eq!(metrics.utilization, 0.0);
        assert_eq!(metrics.per_resource.len(), 1);
        assert_eq!(metrics.per_resource[0].scheduled_task_count, 0);
    }

    #[test]
    fn priority_stats_match_known_values() {
        let problem = problem_with_priorities(&[(1, 10.0), (2, 20.0), (3, 30.0), (4, 40.0)]);
        let mut schedule = Schedule::new();
        place(&mut schedule, 1, 0.0, 0.01);
        place(&mut schedule, 2, 0.02, 0.03);
        place(&mut schedule, 3, 0.04, 0.05);
        place(&mut schedule, 4, 0.06, 0.07);
        let h = horizon(0.0, 1.0);

        let metrics = ScheduleMetrics::compute(&schedule, &problem, &h, &MetricsContext::default());

        assert_eq!(metrics.scheduled_task_count, 4);
        assert_eq!(metrics.total_task_count, 4);
        assert!((metrics.completion_ratio - 1.0).abs() < 1e-12);
        assert_eq!(metrics.priority.count, 4);
        assert!((metrics.priority.sum - 100.0).abs() < 1e-9);
        assert!((metrics.priority.min - 10.0).abs() < 1e-9);
        assert!((metrics.priority.max - 40.0).abs() < 1e-9);
        assert!((metrics.priority.mean - 25.0).abs() < 1e-9);
        // Population stddev of [10,20,30,40] = sqrt(125) ≈ 11.1803
        assert!((metrics.priority.std - 125.0_f64.sqrt()).abs() < 1e-9);
        assert!((metrics.priority.p25 - 17.5).abs() < 1e-9);
        assert!((metrics.priority.p50 - 25.0).abs() < 1e-9);
        assert!((metrics.priority.p75 - 32.5).abs() < 1e-9);
        assert!((metrics.priority.p90 - 37.0).abs() < 1e-9);
    }

    #[test]
    fn missing_priority_defaults_to_zero() {
        let mut tasks = vec![build_task(1, 600.0, Some(10.0))];
        tasks.push(build_task(2, 600.0, None));
        let block = SchedulingBlock::from_tasks(SchedulingBlockId(1), tasks).unwrap();
        let problem = SchedulingProblem::from_blocks(vec![block]).unwrap();

        let mut schedule = Schedule::new();
        place(&mut schedule, 1, 0.0, 0.01);
        place(&mut schedule, 2, 0.02, 0.03);
        let h = horizon(0.0, 1.0);

        let metrics = ScheduleMetrics::compute(&schedule, &problem, &h, &MetricsContext::default());

        assert!((metrics.priority.sum - 10.0).abs() < 1e-9);
        assert!((metrics.priority.min - 0.0).abs() < 1e-9);
        assert!((metrics.priority.max - 10.0).abs() < 1e-9);
    }

    #[test]
    fn fragmentation_counts_internal_gaps_only() {
        let problem = problem_with_priorities(&[(1, 1.0), (2, 1.0), (3, 1.0)]);
        let mut schedule = Schedule::new();
        // Place: [0, 1) day, then gap, then [2, 3), then gap, then [5, 6).
        place(&mut schedule, 1, 0.0, 1.0);
        place(&mut schedule, 2, 2.0, 3.0);
        place(&mut schedule, 3, 5.0, 6.0);
        let h = horizon(0.0, 10.0);

        let metrics = ScheduleMetrics::compute(&schedule, &problem, &h, &MetricsContext::default());

        // Two internal gaps: 1 day and 2 days.
        assert_eq!(metrics.fragmentation.gap_count, 2);
        assert!((metrics.fragmentation.gap_total_sec - 3.0 * SECONDS_PER_DAY).abs() < 1e-6);
        assert!((metrics.fragmentation.largest_gap_sec - 2.0 * SECONDS_PER_DAY).abs() < 1e-6);
        // available = horizon = 10 days = 864000 s; index = 3/10 = 0.3
        assert!((metrics.fragmentation.fragmentation_index - 0.3).abs() < 1e-9);
    }

    #[test]
    fn fragmentation_is_zero_for_back_to_back_placements() {
        let problem = problem_with_priorities(&[(1, 1.0), (2, 1.0)]);
        let mut schedule = Schedule::new();
        place(&mut schedule, 1, 0.0, 1.0);
        place(&mut schedule, 2, 1.0, 2.0);
        let h = horizon(0.0, 5.0);

        let metrics = ScheduleMetrics::compute(&schedule, &problem, &h, &MetricsContext::default());
        assert_eq!(metrics.fragmentation.gap_count, 0);
        assert_eq!(metrics.fragmentation.gap_total_sec, 0.0);
        assert_eq!(metrics.fragmentation.fragmentation_index, 0.0);
    }

    #[test]
    fn utilization_uses_available_override_when_set() {
        let problem = problem_with_priorities(&[(1, 1.0)]);
        let mut schedule = Schedule::new();
        place(&mut schedule, 1, 0.0, 1.0); // 1 day = 86400 s
        let h = horizon(0.0, 10.0);

        // horizon-based: 86400 / 864000 = 0.1
        let m1 = ScheduleMetrics::compute(&schedule, &problem, &h, &MetricsContext::default());
        assert!((m1.utilization - 0.1).abs() < 1e-9);

        // override available time to 1 day -> utilization = 1.0
        let ctx = MetricsContext::default().with_available_time_sec(SECONDS_PER_DAY);
        let m2 = ScheduleMetrics::compute(&schedule, &problem, &h, &ctx);
        assert!((m2.utilization - 1.0).abs() < 1e-9);
        assert!((m2.available_time_sec - SECONDS_PER_DAY).abs() < 1e-9);
    }

    #[test]
    fn per_resource_uses_telescope_name_when_available() {
        // No telescope set: should use the override fallback.
        let problem = problem_with_priorities(&[(1, 1.0)]);
        let mut schedule = Schedule::new();
        place(&mut schedule, 1, 0.0, 1.0);
        let h = horizon(0.0, 10.0);
        let metrics = ScheduleMetrics::compute(&schedule, &problem, &h, &MetricsContext::default());
        assert_eq!(metrics.per_resource.len(), 1);
        assert_eq!(metrics.per_resource[0].resource_id, "resource-0");
        assert_eq!(metrics.per_resource[0].scheduled_task_count, 1);
        assert!((metrics.per_resource[0].scheduled_time_sec - SECONDS_PER_DAY).abs() < 1e-6);
    }

    #[test]
    fn composite_score_combines_normalized_terms() {
        let problem = problem_with_priorities(&[(1, 10.0), (2, 10.0)]);
        let mut schedule = Schedule::new();
        place(&mut schedule, 1, 0.0, 1.0);
        place(&mut schedule, 2, 1.0, 2.0); // back to back, no gaps
        let h = horizon(0.0, 2.0); // perfectly utilized, no fragmentation

        let ctx = MetricsContext::default().with_ranking(RankingWeights::default());
        let metrics = ScheduleMetrics::compute(&schedule, &problem, &h, &ctx);

        // completion=1, priority normalized to 1 (sum/count*max = 20/(2*10)=1),
        // utilization=1 (2 days/2 days), fragmentation index=0 -> term=1.
        // Weighted average with equal weights -> 1.0.
        assert!((metrics.composite_rank_score - 1.0).abs() < 1e-12);
    }

    #[test]
    fn composite_score_handles_zero_weights_gracefully() {
        let problem = problem_with_priorities(&[(1, 1.0)]);
        let mut schedule = Schedule::new();
        place(&mut schedule, 1, 0.0, 1.0);
        let h = horizon(0.0, 1.0);

        let ctx = MetricsContext::default().with_ranking(RankingWeights {
            completion: 0.0,
            priority: 0.0,
            utilization: 0.0,
            fragmentation: 0.0,
        });
        let metrics = ScheduleMetrics::compute(&schedule, &problem, &h, &ctx);
        assert_eq!(metrics.composite_rank_score, 0.0);
    }

    #[test]
    fn percentile_returns_zero_for_empty_input() {
        assert_eq!(percentile(&[], 0.5), 0.0);
        let stats = PriorityStats::from_values(&[]);
        assert_eq!(stats, PriorityStats::default());
    }

    #[test]
    fn percentile_uses_linear_interpolation() {
        let values = [10.0, 20.0, 30.0, 40.0];
        assert!((percentile(&values, 0.25) - 17.5).abs() < 1e-9);
        assert!((percentile(&values, 0.50) - 25.0).abs() < 1e-9);
        assert!((percentile(&values, 0.75) - 32.5).abs() < 1e-9);
        assert!((percentile(&values, 0.90) - 37.0).abs() < 1e-9);
    }
}
