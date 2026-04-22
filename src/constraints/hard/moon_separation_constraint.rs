use super::super::expr::Constraint;
use crate::error::ScheduleError;
use crate::task::IcrsTarget;
use crate::time::{Period, PeriodSet};
use qtty::{Degree, Degrees};
use siderust::calculus::lunar::meeus_ch47::moon_position_meeus_ch47;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;
use siderust::coordinates::frames::ICRS;
use siderust::coordinates::spherical::Direction;
use siderust::time::{JulianDate, MJD};

/// The angular separation between the task target and the Moon must be at least
/// `min_separation` (degrees) at the midpoint of the candidate interval.
#[derive(Debug, Clone)]
pub struct MoonSeparationConstraint {
    pub min_separation: Degrees,
}

impl Constraint for MoonSeparationConstraint {
    fn check(
        &self,
        timeline: &Period<MJD>,
        _location: Option<&Geodetic<ECEF>>,
        target: Option<&IcrsTarget>,
    ) -> Result<PeriodSet<MJD>, ScheduleError> {
        let target = target.ok_or(ScheduleError::MissingTarget)?;

        let mid_jd = JulianDate::new((timeline.start.value() + timeline.end.value()) / 2.0);

        let moon = moon_position_meeus_ch47(mid_jd);

        let moon_dir = Direction::<ICRS>::new_raw(moon.dec.to::<Degree>(), moon.ra.to::<Degree>());

        let sep = target.angular_separation(&moon_dir);

        if sep >= self.min_separation {
            Ok(PeriodSet::from_periods(vec![Period::new(
                timeline.start,
                timeline.end,
            )]))
        } else {
            Ok(PeriodSet::new())
        }
    }
}
