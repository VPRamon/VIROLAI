use super::candidate::EstCandidate;
use crate::time::{MJD, Time};
use std::cmp::Ordering;

/// Sort EST candidates by their current EST metadata using a total order.
///
/// Queue ordering is recomputed from scratch on every refresh so candidates
/// can move freely between schedulable and impossible states.
pub fn sort_candidates(candidates: &mut [EstCandidate<'_>], priority_at: Time<MJD>) {
    candidates.sort_by(|left, right| compare_candidates(left, right, priority_at));
}

/// Comparator implementing the EST queue's total ordering.
///
/// Ordering keys:
/// 1. schedulable candidates before impossible ones,
/// 2. earliest feasible start,
/// 3. lower flexibility,
/// 4. higher soft-constraint score,
/// 5. stable task id order.
pub fn compare_candidates(
    left: &EstCandidate<'_>,
    right: &EstCandidate<'_>,
    priority_at: Time<MJD>,
) -> Ordering {
    left.is_impossible()
        .cmp(&right.is_impossible())
        .then_with(|| cmp_optional_time(left.est, right.est))
        .then_with(|| cmp_f64_asc(left.flexibility, right.flexibility))
        .then_with(|| cmp_f64_desc(left.priority(priority_at), right.priority(priority_at)))
        .then_with(|| left.task_id().0.cmp(&right.task_id().0))
}

/// Compare optional times, treating `Some` as earlier/better than `None`.
fn cmp_optional_time(left: Option<Time<MJD>>, right: Option<Time<MJD>>) -> Ordering {
    match (left, right) {
        (Some(a), Some(b)) => cmp_f64_asc(a.value(), b.value()),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Compare floating-point values in ascending order with total ordering.
fn cmp_f64_asc(left: f64, right: f64) -> Ordering {
    left.total_cmp(&right)
}

/// Compare floating-point values in descending order with total ordering.
fn cmp_f64_desc(left: f64, right: f64) -> Ordering {
    right.total_cmp(&left)
}
