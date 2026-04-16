use super::expr::Constraint;
use crate::task::IcrsTarget;
use crate::time::{Period, PeriodSet};
use qtty::Degrees;
use siderust::calculus::azimuth::{AzimuthProvider, AzimuthQuery};
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::time::MJD;

/// The target azimuth must remain within `[min, max]` for the
/// whole candidate interval.
///
/// Wrap-around ranges are allowed by siderust semantics: when
/// `min > max`, the band crosses North (0°).
#[derive(Debug, Clone)]
pub struct AzimuthConstraint {
    /// Lower azimuth bound (degrees).
    pub min: Degrees,
    /// Upper azimuth bound (degrees).
    pub max: Degrees,
}

impl Constraint for AzimuthConstraint {
    fn check(
        &self,
        timeline: &Period<MJD>,
        location: Option<&Geodetic<ECEF>>,
        target: Option<&IcrsTarget>,
    ) -> PeriodSet<MJD> {
        assert!(
            target.is_some(),
            "missing target for azimuth constraint evaluation"
        );
        assert!(
            location.is_some(),
            "missing location for azimuth constraint evaluation"
        );

        let target = target.unwrap();
        let site = location.unwrap();

        let window = siderust::time::Interval::new(timeline.start, timeline.end);

        let icrs_dir = siderust::coordinates::spherical::direction::ICRS::new(
            target.azimuth,
            target.polar,
        );

        let query = AzimuthQuery {
            observer: *site,
            window,
            min_azimuth: self.min,
            max_azimuth: self.max,
        };

        let periods = icrs_dir.azimuth_periods(&query);

        PeriodSet::from_periods(periods)
    }
}
