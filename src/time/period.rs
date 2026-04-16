use super::{TempochPeriod, TimeScale};

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
        self.start < other.end && other.start < self.end
    }

    #[inline]
    fn touches(&self, other: &Self) -> bool {
        self.end == other.start || other.end == self.start
    }

    #[inline]
    fn intersects(&self, other: &Self) -> Option<Self> {
        self.intersection(other)
    }

    #[inline]
    fn merge(&self, other: &Self) -> Option<Self> {
        if self.overlaps(other) || self.touches(other) {
            Some(Period::new(
                self.start.min(other.start),
                self.end.max(other.end),
            ))
        } else {
            None
        }
    }
}