use super::super::expr::Constraint;
use crate::error::ScheduleError;
use crate::task::IcrsTarget;
use crate::time::{Period, PeriodSet};
use qtty::Degrees;
use siderust::bodies::solar_system::Moon;
use siderust::calculus::altitude::{AltitudePeriodsProvider, AltitudeQuery};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::MJD;

/// The Moon altitude must remain within `[min, max]` (degrees) for the whole
/// candidate interval.
#[derive(Debug, Clone, Copy)]
pub struct MoonAltitudeConstraint {
    /// Minimum allowed Moon altitude above the horizon.
    pub min: Degrees,
    /// Maximum allowed Moon altitude above the horizon.
    pub max: Degrees,
}

impl Constraint for MoonAltitudeConstraint {
    fn check(
        &self,
        timeline: &Period<MJD>,
        location: Option<&Geodetic<ECEF>>,
        _target: Option<&IcrsTarget>,
    ) -> Result<PeriodSet<MJD>, ScheduleError> {
        if self.min > self.max {
            return Err(ScheduleError::InvalidBounds(format!(
                "moon altitude min ({}) > max ({})",
                self.min, self.max
            )));
        }

        let site = location.ok_or(ScheduleError::MissingLocation)?;

        let window = siderust::time::Interval::new(timeline.start, timeline.end);
        let query = AltitudeQuery {
            observer: *site,
            window,
            min_altitude: self.min,
            max_altitude: self.max,
        };

        let periods = Moon.altitude_periods(&query);

        Ok(PeriodSet::from_periods(periods))
    }
}
