use super::candidate::EstCandidate;
use crate::time::{MJD, Time};
use std::cmp::Ordering;

/// Sort EST candidates by earliest start time and flexibility while
/// preserving feasibility, endangered/flexible priority rules,
/// and original order for equal keys.
pub fn sort_candidates(
    candidates: &mut [EstCandidate<'_>],
    endangered_threshold: u32,
    priority_at: Time<MJD>,
) {
    candidates
        .sort_by(|left, right| compare_candidates(left, right, endangered_threshold, priority_at));
}

/// Comparator implementing EST candidate ordering rules.
pub fn compare_candidates(
    left: &EstCandidate<'_>,
    right: &EstCandidate<'_>,
    endangered_threshold: u32,
    priority_at: Time<MJD>,
) -> Ordering {
    left.is_impossible()
        .cmp(&right.is_impossible())
        .then_with(|| compare_by_kind(left, right, endangered_threshold, priority_at))
}

fn compare_same_kind(
    left: &EstCandidate<'_>,
    right: &EstCandidate<'_>,
    priority_at: Time<MJD>,
) -> Ordering {
    cmp_optional_time(left.est, right.est)
        .then_with(|| cmp_f64_desc(left.priority(priority_at), right.priority(priority_at)))
        .then_with(|| cmp_f64_asc(left.flexibility, right.flexibility))
        .then_with(|| left.task_id().0.cmp(&right.task_id().0))
}

fn compare_by_kind(
    left: &EstCandidate<'_>,
    right: &EstCandidate<'_>,
    endangered_threshold: u32,
    priority_at: Time<MJD>,
) -> Ordering {
    match (
        left.is_endangered(endangered_threshold),
        right.is_endangered(endangered_threshold),
    ) {
        (true, true) | (false, false) => compare_same_kind(left, right, priority_at),
        (false, true) => compare_flexible_and_endangered(left, right),
        (true, false) => compare_flexible_and_endangered(right, left).reverse(),
    }
}

fn compare_flexible_and_endangered(
    flexible: &EstCandidate<'_>,
    endangered: &EstCandidate<'_>,
) -> Ordering {
    if flexible_can_precede_endangered(flexible, endangered) {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

fn flexible_can_precede_endangered(
    flexible: &EstCandidate<'_>,
    endangered: &EstCandidate<'_>,
) -> bool {
    let (Some(flexible_est), Some(endangered_est), Some(endangered_deadline)) =
        (flexible.est, endangered.est, endangered.deadline)
    else {
        return false;
    };

    flexible_est < endangered_est && (flexible_est + flexible.duration()) <= endangered_deadline
}

fn cmp_optional_time(left: Option<Time<MJD>>, right: Option<Time<MJD>>) -> Ordering {
    match (left, right) {
        (Some(a), Some(b)) => cmp_f64_asc(a.value(), b.value()),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn cmp_f64_asc(left: f64, right: f64) -> Ordering {
    left.partial_cmp(&right).unwrap_or(Ordering::Equal)
}

fn cmp_f64_desc(left: f64, right: f64) -> Ordering {
    right.partial_cmp(&left).unwrap_or(Ordering::Equal)
}
