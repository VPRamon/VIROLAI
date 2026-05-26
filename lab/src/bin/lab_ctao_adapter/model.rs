//! Minimal CTAO input and scheduler output schema types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub(crate) struct OutSchedulingProblem {
    pub(crate) resources: Vec<OutTelescope>,
    pub(crate) schedule_time_window: OutTimeWindow,
    pub(crate) scheduling_blocks: Vec<OutBlock>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutLocation {
    pub(crate) longitude_deg: f64,
    pub(crate) latitude_deg: f64,
    pub(crate) height_m: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutTelescope {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) location: OutLocation,
    pub(crate) hard_constraints: OutResourceHardConstraints,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutResourceHardConstraints {
    pub(crate) night_time: OutNightTimeConstraint,
    pub(crate) moon_altitude: OutMoonAltitudeConstraint,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutNightTimeConstraint {
    pub(crate) twilight: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutMoonAltitudeConstraint {
    pub(crate) min_deg: f64,
    pub(crate) max_deg: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutBlock {
    pub(crate) id: u64,
    pub(crate) tasks: Vec<OutTask>,
    pub(crate) dependencies: Vec<OutDependency>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutTask {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) requested_duration_sec: f64,
    pub(crate) target: OutTarget,
    pub(crate) hard_constraints: OutHardConstraints,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) soft_constraints: Option<OutSoftConstraints>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutTarget {
    pub(crate) ra_deg: f64,
    pub(crate) dec_deg: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutHardConstraints {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) altitude_min_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) altitude_max_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) azimuth_min_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) azimuth_max_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) time_window: Option<OutTimeWindow>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutSoftConstraints {
    pub(crate) priority: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutTimeWindow {
    pub(crate) start_mjd_utc: f64,
    pub(crate) end_mjd_utc: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct OutDependency {}

#[derive(Debug, Deserialize)]
pub(crate) struct CtaoFile {
    #[serde(rename = "SchedulingBlock")]
    pub(crate) scheduling_block: Vec<Value>,
}
