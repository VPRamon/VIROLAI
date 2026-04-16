use super::super::expr::Constraint;
use crate::task::IcrsTarget;
use crate::time::{Period, PeriodSet};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::MJD;

/// The candidate interval must be fully contained within `[start, end)`.
#[derive(Debug, Clone)]
pub struct TimeConstraint {
    pub window: Period<MJD>,
}

pub type TimeWindowConstraint = TimeConstraint;

impl Constraint for TimeConstraint {
    fn check(
        &self,
        timeline: &Period<MJD>,
        _location: Option<&Geodetic<ECEF>>,
        _target: Option<&IcrsTarget>,
    ) -> PeriodSet<MJD> {
        if let Some(overlap) = self.window.intersection(timeline) {
            PeriodSet::from_periods(vec![overlap])
        } else {
            PeriodSet::new()
        }
    }
}
