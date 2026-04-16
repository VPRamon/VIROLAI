use super::expr::Constraint;
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
    ) -> PeriodSet<MJD> {
        assert!(
            self.min <= self.max,
            "invalid moon altitude range: min is greater than max"
        );

        assert!(
            location.is_some(),
            "missing location for moon-altitude constraint evaluation"
        );

        let site = location.unwrap();

        let window = siderust::time::Interval::new(timeline.start, timeline.end);
        let query = AltitudeQuery {
            observer: *site,
            window,
            min_altitude: self.min,
            max_altitude: self.max,
        };

        let periods = Moon.altitude_periods(&query);

        PeriodSet::from_periods(periods)
    }
}
