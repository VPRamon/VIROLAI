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
