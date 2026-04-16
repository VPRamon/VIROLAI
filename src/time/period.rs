use super::{interval, TempochPeriod, TimeScale};

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