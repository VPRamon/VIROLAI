use super::{TempochPeriod, TimeScale, interval};

pub type Period<S> = TempochPeriod<S>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PeriodError {
    InvalidRange,
}

pub trait PeriodExt: Sized {
    fn overlaps(&self, other: &Self) -> bool;
    fn touches(&self, other: &Self) -> bool;
    fn intersects(&self, other: &Self) -> Option<Self>;
    fn merge(&self, other: &Self) -> Option<Self>;
}

impl<S: TimeScale> PeriodExt for Period<S> {
    #[inline]
    fn overlaps(&self, other: &Self) -> bool {
        interval::overlaps(self, other)
    }

    #[inline]
    fn touches(&self, other: &Self) -> bool {
        interval::touches(self, other)
    }

    #[inline]
    fn intersects(&self, other: &Self) -> Option<Self> {
        self.intersection(other)
    }

    #[inline]
    fn merge(&self, other: &Self) -> Option<Self> {
        interval::merge_periods(self, other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{MJD, Time};

    fn p(start: f64, end: f64) -> Period<MJD> {
        Period::new(Time::<MJD>::new(start), Time::<MJD>::new(end))
    }

    #[test]
    fn overlaps_and_touches_delegate_to_interval_helpers() {
        assert!(p(1.0, 3.0).overlaps(&p(2.0, 4.0)));
        assert!(!p(1.0, 2.0).overlaps(&p(2.0, 3.0)));

        assert!(p(1.0, 2.0).touches(&p(2.0, 3.0)));
        assert!(!p(1.0, 2.0).touches(&p(2.1, 3.0)));
    }

    #[test]
    fn intersects_and_merge_delegate_consistently() {
        assert_eq!(p(1.0, 4.0).intersects(&p(3.0, 6.0)), Some(p(3.0, 4.0)));
        assert_eq!(p(1.0, 2.0).intersects(&p(2.0, 3.0)), None);

        assert_eq!(p(1.0, 2.0).merge(&p(2.0, 4.0)), Some(p(1.0, 4.0)));
        assert_eq!(p(1.0, 2.0).merge(&p(3.0, 4.0)), None);
    }
}
