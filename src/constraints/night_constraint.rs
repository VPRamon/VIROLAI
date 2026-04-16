use super::expr::{Constraint, ConstraintResult};
use crate::error::ScheduleError;
use crate::task::IcrsTarget;
use crate::time::{Period, PeriodSet};
use siderust::bodies::solar_system::Sun;
use siderust::calculus::altitude::AltitudePeriodsProvider;
use siderust::calculus::solar::Twilight;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::MJD;

/// The candidate interval must occur during night.
///
/// Night is defined as periods where the Sun is below the selected twilight
/// threshold from siderust (`Twilight::Civil`, `Twilight::Nautical`,
/// `Twilight::Astronomical`, etc.).
#[derive(Debug, Clone, Copy)]
pub struct NightConstraint {
    pub twilight: Twilight,
}

impl Constraint for NightConstraint {
    fn check(
        &self,
        timeline: &Period<MJD>,
        location: Option<&Geodetic<ECEF>>,
        _target: Option<&IcrsTarget>,
    ) -> ConstraintResult {
        let site = location.ok_or_else(|| {
            ScheduleError::ConstraintViolation(
                "missing location for night constraint evaluation".into(),
            )
        })?;

        let window = siderust::time::Interval::new(timeline.start, timeline.end);
        let threshold = qtty::Degrees::from(self.twilight);
        let periods = Sun.below_threshold(*site, window, threshold);

        Ok(PeriodSet::from_periods(periods))
    }
}
