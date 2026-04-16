use super::{Period, PeriodExt, Time, TimeScale};
use std::cmp::Ordering;

/// Shared interval abstraction for time-based concepts in this crate.
pub(crate) trait IntervalLike<S: TimeScale> {
    fn start(&self) -> Time<S>;
    fn end(&self) -> Time<S>;
}

impl<S: TimeScale> IntervalLike<S> for Period<S> {
    fn start(&self) -> Time<S> {
        self.start
    }

    fn end(&self) -> Time<S> {
        self.end
    }
}

#[inline]
pub(crate) fn overlaps<S: TimeScale, A: IntervalLike<S>, B: IntervalLike<S>>(a: &A, b: &B) -> bool {
    a.start() < b.end() && b.start() < a.end()
}

#[inline]
pub(crate) fn touches<S: TimeScale, A: IntervalLike<S>, B: IntervalLike<S>>(a: &A, b: &B) -> bool {
    a.end() == b.start() || b.end() == a.start()
}

#[inline]
pub(crate) fn contains_point<S: TimeScale, I: IntervalLike<S>>(interval: &I, t: Time<S>) -> bool {
    interval.start() <= t && t < interval.end()
}

pub(crate) fn merge_periods<S: TimeScale>(a: &Period<S>, b: &Period<S>) -> Option<Period<S>> {
    if overlaps(a, b) || touches(a, b) {
        Some(Period::new(a.start.min(b.start), a.end.max(b.end)))
    } else {
        None
    }
}

pub(crate) fn normalize_periods<S: TimeScale>(mut periods: Vec<Period<S>>) -> Vec<Period<S>> {
    if periods.is_empty() {
        return periods;
    }

    periods.sort_by(|a, b| {
        let by_start = a.start.partial_cmp(&b.start).unwrap_or(Ordering::Equal);
        if by_start != Ordering::Equal {
            return by_start;
        }

        a.end.partial_cmp(&b.end).unwrap_or(Ordering::Equal)
    });

    let mut out = Vec::with_capacity(periods.len());
    for period in periods {
        push_merged(&mut out, period);
    }

    out
}

pub(crate) fn union_periods<S: TimeScale>(
    left: &[Period<S>],
    right: &[Period<S>],
) -> Vec<Period<S>> {
    let mut out = Vec::with_capacity(left.len() + right.len());
    let mut i = 0;
    let mut j = 0;

    while i < left.len() && j < right.len() {
        let next = if left[i].start <= right[j].start {
            let period = left[i];
            i += 1;
            period
        } else {
            let period = right[j];
            j += 1;
            period
        };

        push_merged(&mut out, next);
    }

    while i < left.len() {
        let period = left[i];
        i += 1;
        push_merged(&mut out, period);
    }

    while j < right.len() {
        let period = right[j];
        j += 1;
        push_merged(&mut out, period);
    }

    out
}

pub(crate) fn intersection_periods<S: TimeScale>(
    left: &[Period<S>],
    right: &[Period<S>],
) -> Vec<Period<S>> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut j = 0;

    while i < left.len() && j < right.len() {
        let a = left[i];
        let b = right[j];

        if let Some(intersection) = a.intersects(&b) {
            out.push(intersection);
        }

        if a.end <= b.end {
            i += 1;
        } else {
            j += 1;
        }
    }

    out
}

fn push_merged<S: TimeScale>(out: &mut Vec<Period<S>>, period: Period<S>) {
    if let Some(last) = out.last_mut()
        && let Some(merged) = merge_periods(last, &period)
    {
        *last = merged;
        return;
    }

    out.push(period);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::MJD;

    fn p(start: f64, end: f64) -> Period<MJD> {
        Period::new(Time::<MJD>::new(start), Time::<MJD>::new(end))
    }

    #[test]
    fn overlaps_and_touches_follow_half_open_semantics() {
        assert!(overlaps(&p(1.0, 3.0), &p(2.0, 4.0)));
        assert!(!overlaps(&p(1.0, 2.0), &p(2.0, 3.0)));

        assert!(touches(&p(1.0, 2.0), &p(2.0, 3.0)));
        assert!(!touches(&p(1.0, 2.0), &p(2.1, 3.0)));
    }

    #[test]
    fn contains_point_is_start_inclusive_end_exclusive() {
        let period = p(10.0, 20.0);
        assert!(contains_point(&period, Time::<MJD>::new(10.0)));
        assert!(contains_point(&period, Time::<MJD>::new(19.999)));
        assert!(!contains_point(&period, Time::<MJD>::new(20.0)));
    }

    #[test]
    fn merge_periods_merges_overlaps_and_touches_only() {
        assert_eq!(merge_periods(&p(0.0, 2.0), &p(1.0, 3.0)), Some(p(0.0, 3.0)));
        assert_eq!(merge_periods(&p(0.0, 2.0), &p(2.0, 3.0)), Some(p(0.0, 3.0)));
        assert_eq!(merge_periods(&p(0.0, 2.0), &p(3.0, 4.0)), None);
    }

    #[test]
    fn normalize_periods_handles_empty_and_sorts_and_merges() {
        let empty: Vec<Period<MJD>> = Vec::new();
        assert!(normalize_periods(empty).is_empty());

        let normalized =
            normalize_periods(vec![p(5.0, 8.0), p(5.0, 6.0), p(8.0, 9.0), p(1.0, 2.0)]);

        assert_eq!(normalized, vec![p(1.0, 2.0), p(5.0, 9.0)]);
    }

    #[test]
    fn union_periods_merges_and_preserves_remaining_tails() {
        let left = vec![p(0.0, 2.0), p(5.0, 8.0), p(20.0, 22.0)];
        let right = vec![p(2.0, 3.0), p(7.0, 10.0), p(30.0, 31.0)];

        let union = union_periods(&left, &right);

        assert_eq!(
            union,
            vec![p(0.0, 3.0), p(5.0, 10.0), p(20.0, 22.0), p(30.0, 31.0)]
        );
    }

    #[test]
    fn intersection_periods_exercises_both_pointer_advancement_paths() {
        let left = vec![p(0.0, 5.0), p(7.0, 10.0)];
        let right = vec![p(3.0, 8.0), p(9.0, 12.0)];

        let intersection = intersection_periods(&left, &right);

        assert_eq!(intersection, vec![p(3.0, 5.0), p(7.0, 8.0), p(9.0, 10.0)]);
    }
}
