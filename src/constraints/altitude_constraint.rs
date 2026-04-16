use super::expr::{Constraint, ConstraintResult};
use crate::error::ScheduleError;
use crate::period::Period;
use crate::period_set::PeriodSet;
use crate::task::IcrsTarget;
use qtty::Degrees;
use siderust::calculus::altitude::AltitudePeriodsProvider;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::MJD;

/// The target must remain within `[min, max]` (degrees) for
/// the whole candidate interval.
///
/// The check is performed by querying siderust for the altitude periods where
/// the target exceeds the threshold and verifying that the candidate interval
/// is fully covered.
#[derive(Debug, Clone)]
pub struct AltitudeConstraint {
    /// Minimum allowed altitude above the horizon.
    pub min: Degrees,
    /// Maximum allowed altitude above the horizon.
    pub max: Degrees,
}

impl Constraint for AltitudeConstraint {
    fn check(
        &self,
        timeline: &Period<MJD>,
        location: Option<&Geodetic<ECEF>>,
        target: Option<&IcrsTarget>,
    ) -> ConstraintResult {
        if self.min > self.max {
            return Err(ScheduleError::ConstraintViolation(
                "invalid altitude range: min is greater than max".into(),
            ));
        }

        let target = target.ok_or_else(|| {
            ScheduleError::ConstraintViolation(
                "missing target for altitude constraint evaluation".into(),
            )
        })?;
        let site = location.ok_or_else(|| {
            ScheduleError::ConstraintViolation(
                "missing location for altitude constraint evaluation".into(),
            )
        })?;

        let window = siderust::time::Interval::new(timeline.start, timeline.end);

        let icrs_dir = siderust::coordinates::spherical::direction::ICRS::new(
            target.azimuth,
            target.polar,
        );

        let query = siderust::calculus::altitude::AltitudeQuery {
            observer: *site,
            window,
            min_altitude: self.min,
            max_altitude: self.max,
        };

        let periods = icrs_dir.altitude_periods(&query);

        Ok(PeriodSet::from_periods(periods))
    }
}
