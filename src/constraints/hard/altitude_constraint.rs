use super::super::expr::Constraint;
use crate::task::IcrsTarget;
use crate::time::{Period, PeriodSet};
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
    ) -> PeriodSet<MJD> {
        assert!(
            self.min <= self.max,
            "invalid altitude range: min is greater than max"
        );

        assert!(
            target.is_some(),
            "missing target for altitude constraint evaluation"
        );
        assert!(
            location.is_some(),
            "missing location for altitude constraint evaluation"
        );

        let target = target.unwrap();
        let site = location.unwrap();

        let window = siderust::time::Interval::new(timeline.start, timeline.end);

        let icrs_dir =
            siderust::coordinates::spherical::direction::ICRS::new(target.azimuth, target.polar);

        let query = siderust::calculus::altitude::AltitudeQuery {
            observer: *site,
            window,
            min_altitude: self.min,
            max_altitude: self.max,
        };

        let periods = icrs_dir.altitude_periods(&query);

        PeriodSet::from_periods(periods)
    }
}
