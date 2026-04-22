use super::candidate::EstCandidate;
use crate::time::{MJD, TaskId, Time};
use std::cmp::Ordering;

#[derive(Clone, Copy)]
struct OrderingKey {
    effective_est: Option<Time<MJD>>,
    is_endangered: bool,
    est: Option<Time<MJD>>,
    flexibility: f64,
    priority: f64,
    task_id: TaskId,
}

/// Sort EST candidates by their current EST metadata using a total order.
///
/// Before sorting, a temporary ordering key is computed for each schedulable
/// candidate. Non-endangered candidates that would obstruct an endangered one
/// have their derived `effective_est` promoted to the endangered candidate's
/// EST so they are deferred behind it.
///
/// Queue ordering is recomputed from scratch on every refresh so candidates
/// can move freely between schedulable and impossible states.
pub fn sort_candidates(
    candidates: &mut [EstCandidate<'_>],
    priority_at: Time<MJD>,
    threshold: u32,
) {
    let keys = build_ordering_keys(candidates, priority_at, threshold);
    let mut keyed_candidates = candidates
        .iter()
        .cloned()
        .zip(keys)
        .map(|(candidate, key)| (key, candidate))
        .collect::<Vec<_>>();

    keyed_candidates
        .sort_by(|(left_key, _), (right_key, _)| compare_ordering_keys(left_key, right_key));

    for (slot, (_, candidate)) in candidates.iter_mut().zip(keyed_candidates.into_iter()) {
        *slot = candidate;
    }
}

/// Build the temporary ordering key for every candidate in the current queue.
///
/// * Impossible candidates → `None`.
/// * Endangered candidates → `effective_est = est`.
/// * Non-endangered candidates → `effective_est` starts as `est`; for each
///   endangered candidate E whose EST falls within `[N.est, N.est + N.dur)`,
///   `effective_est` is promoted to `max(effective_est, E.est)`.
fn build_ordering_keys(
    candidates: &[EstCandidate<'_>],
    priority_at: Time<MJD>,
    threshold: u32,
) -> Vec<OrderingKey> {
    let endangered_ests: Vec<Time<MJD>> = candidates
        .iter()
        .filter(|c| !c.is_impossible() && c.is_endangered(threshold))
        .filter_map(|c| c.est)
        .collect();

    candidates
        .iter()
        .map(|candidate| OrderingKey {
            effective_est: derive_effective_est(candidate, &endangered_ests, threshold),
            is_endangered: candidate.is_endangered(threshold),
            est: candidate.est,
            flexibility: candidate.flexibility,
            priority: candidate.priority(priority_at),
            task_id: candidate.task_id(),
        })
        .collect()
}

fn derive_effective_est(
    candidate: &EstCandidate<'_>,
    endangered_ests: &[Time<MJD>],
    threshold: u32,
) -> Option<Time<MJD>> {
    if candidate.is_impossible() {
        return None;
    }

    let est = candidate.est?;
    if candidate.is_endangered(threshold) {
        return Some(est);
    }

    let est_days = est.value();
    let duration_days = candidate.duration().value();
    let mut effective_est = est;
    for &endangered_est in endangered_ests {
        let endangered_est_days = endangered_est.value();
        if est_days <= endangered_est_days
            && est_days + duration_days > endangered_est_days
            && endangered_est_days > effective_est.value()
        {
            effective_est = endangered_est;
        }
    }

    Some(effective_est)
}

/// Comparator implementing the EST queue's total ordering.
///
/// Ordering keys:
/// 1. schedulable candidates before impossible ones,
/// 2. `effective_est` ascending (lower = earlier = better),
/// 3. endangered candidates before non-endangered ones at the same `effective_est`,
/// 4. original `est` ascending,
/// 5. lower flexibility,
/// 6. higher soft-constraint score,
/// 7. stable task id order.
pub fn compare_candidates(
    left: &EstCandidate<'_>,
    right: &EstCandidate<'_>,
    priority_at: Time<MJD>,
    threshold: u32,
) -> Ordering {
    let pair = [left, right];
    let endangered_ests: Vec<Time<MJD>> = pair
        .iter()
        .copied()
        .filter(|candidate| !candidate.is_impossible() && candidate.is_endangered(threshold))
        .filter_map(|candidate| candidate.est)
        .collect();
    let left_key = OrderingKey {
        effective_est: derive_effective_est(left, &endangered_ests, threshold),
        is_endangered: left.is_endangered(threshold),
        est: left.est,
        flexibility: left.flexibility,
        priority: left.priority(priority_at),
        task_id: left.task_id(),
    };
    let right_key = OrderingKey {
        effective_est: derive_effective_est(right, &endangered_ests, threshold),
        is_endangered: right.is_endangered(threshold),
        est: right.est,
        flexibility: right.flexibility,
        priority: right.priority(priority_at),
        task_id: right.task_id(),
    };

    compare_ordering_keys(&left_key, &right_key)
}

fn compare_ordering_keys(left: &OrderingKey, right: &OrderingKey) -> Ordering {
    left.effective_est
        .is_none()
        .cmp(&right.effective_est.is_none())
        .then_with(|| cmp_optional_time(left.effective_est, right.effective_est))
        .then_with(|| right.is_endangered.cmp(&left.is_endangered))
        .then_with(|| cmp_optional_time(left.est, right.est))
        .then_with(|| cmp_f64_asc(left.flexibility, right.flexibility))
        .then_with(|| cmp_f64_desc(left.priority, right.priority))
        .then_with(|| left.task_id.0.cmp(&right.task_id.0))
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
