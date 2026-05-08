use super::candidate::Candidate;
use crate::time::{MJD, TaskId, Time};
use std::cmp::Ordering;

/// Ordering key used by the pairwise `compare_candidates` helper.
#[derive(Clone, Copy)]
struct OrderingKey {
    effective_est: Option<Time<MJD>>,
    is_endangered: bool,
    est: Option<Time<MJD>>,
    flexibility: f64,
    priority: f64,
    task_id: TaskId,
}

/// Sort EST candidates in-place by their current EST metadata using a total order.
///
/// Populates the cached ordering fields (`effective_est`, `is_endangered_cached`,
/// `priority_at_cursor`) on every candidate, then sorts in-place with
/// [`compare_by_cached`].  No candidate is cloned during sorting.
///
/// Non-endangered candidates that would obstruct an endangered one have their
/// `effective_est` promoted to the endangered candidate's EST so they are
/// deferred behind it.
pub fn sort_candidates(candidates: &mut [Candidate<'_>], priority_at: Time<MJD>, threshold: u32) {
    let endangered_ests: Vec<Time<MJD>> = candidates
        .iter()
        .filter(|c| !c.is_impossible() && c.is_endangered(threshold))
        .filter_map(|c| c.est)
        .collect();

    for c in candidates.iter_mut() {
        c.priority_at_cursor = c.priority(priority_at);
        let is_endangered = !c.is_impossible() && c.is_endangered(threshold);
        c.is_endangered_cached = is_endangered;
        c.effective_est = compute_effective_est(c, is_endangered, &endangered_ests);
    }

    candidates.sort_by(compare_by_cached);
}

/// Compute the effective EST for a candidate given the pre-collected endangered list.
///
/// * Impossible candidates → `None`.
/// * Endangered candidates → `effective_est = est`.
/// * Non-endangered candidates → `effective_est` starts as `est`; for each
///   endangered candidate E whose EST falls within `[c.est, c.est + c.dur)`,
///   `effective_est` is promoted to `max(effective_est, E.est)`.
fn compute_effective_est(
    candidate: &Candidate<'_>,
    is_endangered: bool,
    endangered_ests: &[Time<MJD>],
) -> Option<Time<MJD>> {
    if candidate.is_impossible() {
        return None;
    }
    let est = candidate.est?;
    if is_endangered {
        return Some(est);
    }
    let est_days = est.value();
    let duration_days = candidate.duration().value();
    let mut effective = est;
    for &e in endangered_ests {
        let ed = e.value();
        if est_days <= ed && est_days + duration_days > ed && ed > effective.value() {
            effective = e;
        }
    }
    Some(effective)
}

/// Compare two candidates using their pre-populated cached ordering fields.
///
/// This is the hot-path comparator used during queue sorting; cached fields
/// must be populated before calling this (done by [`sort_candidates`] and
/// [`super::queue::CandidateQueue::refresh`]).
///
/// Ordering keys (identical semantics to the full EST total order):
/// 1. schedulable candidates before impossible ones,
/// 2. `effective_est` ascending,
/// 3. endangered before non-endangered at the same `effective_est`,
/// 4. original `est` ascending,
/// 5. lower flexibility,
/// 6. higher soft-constraint score,
/// 7. lower task id.
pub(crate) fn compare_by_cached(a: &Candidate<'_>, b: &Candidate<'_>) -> Ordering {
    match (a.is_impossible(), b.is_impossible()) {
        (false, true) => return Ordering::Less,
        (true, false) => return Ordering::Greater,
        _ => {}
    }
    let cmp = cmp_optional_time(a.effective_est, b.effective_est);
    if cmp != Ordering::Equal {
        return cmp;
    }
    match (a.is_endangered_cached, b.is_endangered_cached) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    let cmp = cmp_optional_time(a.est, b.est);
    if cmp != Ordering::Equal {
        return cmp;
    }
    let cmp = cmp_f64_asc(a.flexibility, b.flexibility);
    if cmp != Ordering::Equal {
        return cmp;
    }
    let cmp = cmp_f64_desc(a.priority_at_cursor, b.priority_at_cursor);
    if cmp != Ordering::Equal {
        return cmp;
    }
    a.task_id().0.cmp(&b.task_id().0)
}

/// Pairwise comparator for two candidates.
///
/// Computes ordering keys from scratch for both candidates (including the
/// endangered-promotion rule restricted to just this pair).
///
/// **Note:** this comparator is correct for pairwise tests but is **not**
/// suitable for whole-queue endangered-promotion correctness: that rule
/// requires the full set of endangered ESTs from the entire queue, which only
/// [`sort_candidates`] or [`super::queue::CandidateQueue::refresh`] can
/// provide.
pub fn compare_candidates(
    left: &Candidate<'_>,
    right: &Candidate<'_>,
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
    let left_is_endangered = !left.is_impossible() && left.is_endangered(threshold);
    let right_is_endangered = !right.is_impossible() && right.is_endangered(threshold);
    let left_key = OrderingKey {
        effective_est: compute_effective_est(left, left_is_endangered, &endangered_ests),
        is_endangered: left_is_endangered,
        est: left.est,
        flexibility: left.flexibility,
        priority: left.priority(priority_at),
        task_id: left.task_id(),
    };
    let right_key = OrderingKey {
        effective_est: compute_effective_est(right, right_is_endangered, &endangered_ests),
        is_endangered: right_is_endangered,
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
