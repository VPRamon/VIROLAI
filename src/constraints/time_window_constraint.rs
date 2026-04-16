use super::expr::{Constraint, ConstraintResult};
use crate::period::Period;
use crate::period_set::PeriodSet;
use crate::task::IcrsTarget;
use crate::time;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::MJD;

/// The candidate interval must be fully contained within `[start, end)`.
#[derive(Debug, Clone)]
pub struct TimeConstraint {
    pub window: time::Period<MJD>,
}

pub type TimeWindowConstraint = TimeConstraint;

impl Constraint for TimeConstraint {
    fn check(
        &self,
        timeline: &Period<MJD>,
        _location: Option<&Geodetic<ECEF>>,
        _target: Option<&IcrsTarget>,
    ) -> ConstraintResult {
        if let Some(overlap) = self.window.intersection(timeline) {
            Ok(PeriodSet::from_periods(vec![overlap]))
        } else {
            Ok(PeriodSet::new())
        }
    }
}
