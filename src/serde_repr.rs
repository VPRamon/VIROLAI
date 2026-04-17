//! Shared JSON representation types used across the scheduler deserializers.
//!
//! These types mirror the on-disk schema fragments defined in
//! `schemas/hard_constraints.schema.json` and related files. They are kept
//! in one place so that task, telescope, and top-level problem deserializers
//! all agree on the wire format.

use crate::constraints::{
    AltitudeConstraint, AzimuthConstraint, ConstraintBlocks, ConstraintExpr,
    MoonAltitudeConstraint, NightConstraint, TimeConstraint,
};
use crate::time::{MJD, Period, Time};
use qtty::{Degrees, Meters};
use serde::Deserialize;
use siderust::calculus::solar::Twilight;
use siderust::coordinates::centers::Geodetic;
use siderust::coordinates::frames::ECEF;

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct TimeWindowRepr {
    pub start_mjd_utc: f64,
    pub end_mjd_utc: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct LocationRepr {
    pub longitude_deg: f64,
    pub latitude_deg: f64,
    pub height_m: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct NightTimeRepr {
    pub twilight: TwilightRepr,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) enum TwilightRepr {
    Civil,
    Nautical,
    Astronomical,
    Horizon,
    ApparentHorizon,
}

impl From<TwilightRepr> for Twilight {
    fn from(value: TwilightRepr) -> Self {
        match value {
            TwilightRepr::Civil => Twilight::Civil,
            TwilightRepr::Nautical => Twilight::Nautical,
            TwilightRepr::Astronomical => Twilight::Astronomical,
            TwilightRepr::Horizon => Twilight::Horizon,
            TwilightRepr::ApparentHorizon => Twilight::ApparentHorizon,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct MoonAltitudeRepr {
    pub min_deg: f64,
    pub max_deg: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct HardConstraintsRepr {
    pub altitude_min_deg: Option<f64>,
    pub altitude_max_deg: Option<f64>,
    pub azimuth_min_deg: Option<f64>,
    pub azimuth_max_deg: Option<f64>,
    pub time_window: Option<TimeWindowRepr>,
    pub night_time: Option<NightTimeRepr>,
    pub moon_altitude: Option<MoonAltitudeRepr>,
}

pub(crate) fn mjd_period(start: f64, end: f64) -> Result<Period<MJD>, String> {
    if !start.is_finite() || !end.is_finite() {
        return Err("time_window bounds must be finite".to_string());
    }
    if start >= end {
        return Err(format!(
            "time_window start ({start}) must be before end ({end})"
        ));
    }
    Ok(Period::new(Time::<MJD>::new(start), Time::<MJD>::new(end)))
}

pub(crate) fn geodetic_location_from_repr(repr: LocationRepr) -> Result<Geodetic<ECEF>, String> {
    if !repr.longitude_deg.is_finite()
        || !repr.latitude_deg.is_finite()
        || !repr.height_m.is_finite()
    {
        return Err("location coordinates must be finite".to_string());
    }

    Ok(Geodetic::<ECEF>::new(
        Degrees::new(repr.longitude_deg),
        Degrees::new(repr.latitude_deg),
        Meters::new(repr.height_m),
    ))
}

pub(crate) fn hard_constraint_parts_from_repr(
    repr: &HardConstraintsRepr,
) -> Result<(Vec<ConstraintExpr>, Vec<ConstraintExpr>), String> {
    let mut hard_static = Vec::new();
    let mut hard_dynamic = Vec::new();

    if repr.altitude_min_deg.is_some() || repr.altitude_max_deg.is_some() {
        let min = repr.altitude_min_deg.unwrap_or(0.0);
        let max = repr.altitude_max_deg.unwrap_or(90.0);
        if min > max {
            return Err(format!("invalid altitude bounds: min {min} > max {max}"));
        }
        hard_dynamic.push(ConstraintExpr::atom(AltitudeConstraint {
            min: Degrees::new(min),
            max: Degrees::new(max),
        }));
    }

    if repr.azimuth_min_deg.is_some() || repr.azimuth_max_deg.is_some() {
        let min = repr.azimuth_min_deg.unwrap_or(0.0);
        let max = repr.azimuth_max_deg.unwrap_or(360.0);
        hard_dynamic.push(ConstraintExpr::atom(AzimuthConstraint {
            min: Degrees::new(min),
            max: Degrees::new(max),
        }));
    }

    if let Some(tw) = repr.time_window {
        let window = mjd_period(tw.start_mjd_utc, tw.end_mjd_utc)?;
        hard_static.push(ConstraintExpr::atom(TimeConstraint { window }));
    }

    if let Some(night_time) = repr.night_time {
        hard_dynamic.push(ConstraintExpr::atom(NightConstraint {
            twilight: night_time.twilight.into(),
        }));
    }

    if let Some(moon_altitude) = repr.moon_altitude {
        if !moon_altitude.min_deg.is_finite() || !moon_altitude.max_deg.is_finite() {
            return Err("moon altitude bounds must be finite".to_string());
        }
        if moon_altitude.min_deg > moon_altitude.max_deg {
            return Err(format!(
                "invalid moon altitude bounds: min {} > max {}",
                moon_altitude.min_deg, moon_altitude.max_deg
            ));
        }
        if moon_altitude.max_deg > 0.0 {
            return Err(format!(
                "invalid moon altitude max {}: must be <= 0 to enforce below horizon",
                moon_altitude.max_deg
            ));
        }
        hard_dynamic.push(ConstraintExpr::atom(MoonAltitudeConstraint {
            min: Degrees::new(moon_altitude.min_deg),
            max: Degrees::new(moon_altitude.max_deg),
        }));
    }

    Ok((hard_static, hard_dynamic))
}

pub(crate) fn hard_constraint_blocks_from_repr(
    repr: &HardConstraintsRepr,
) -> Result<ConstraintBlocks, String> {
    let (hard_static, hard_dynamic) = hard_constraint_parts_from_repr(repr)?;
    Ok(ConstraintBlocks::new(
        ConstraintExpr::Intersection(hard_static),
        ConstraintExpr::Intersection(hard_dynamic),
    ))
}
