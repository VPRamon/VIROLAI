//! Bounded-context adapter: PhD scheduler model → TSI analysis model.
//!
//! # What this adapter does
//!
//! This adapter translates a PhD scheduling-problem JSON payload into the TSI
//! internal analysis model consumed by the TSI backend.  It is an explicit
//! **bounded-context** translation: the two models serve different purposes and
//! share only a subset of their concepts.
//!
//! ## Preserved
//! - Task identity (`task_id` / `original_task_id`)
//! - Scheduled time intervals (start / end MJD UTC)
//! - Task priority
//! - Per-task azimuth and altitude constraints
//! - Observing-site location (from `resources[0]` or legacy top-level `location`)
//! - Astronomical observability constraints (twilight type, moon altitude bounds)
//! - Block membership tracked as `original_block_id` on each TSI block
//!
//! ## Deliberately dropped
//! - Multi-resource schedules: only `resources[0]` is used; passing more than
//!   one resource is a hard error (see below).
//! - Intra-block dependency edges: TSI has no concept of task ordering within a
//!   block; dependencies are satisfied before export by the scheduler.
//! - Unscheduled tasks: tasks without `scheduled_start/end_mjd_utc` are silently
//!   skipped (they carry no placement information).
//!
//! ## Multi-resource rejection
//! TSI is a single-site analysis tool.  If the payload contains more than one
//! resource the adapter returns a hard error rather than silently discarding
//! data.

use std::sync::Arc;

use anyhow::{Context, bail};
use rayon::prelude::*;
use serde::Deserialize;
use tsi_rust::api::{self, Schedule};
use tsi_rust::models::ModifiedJulianDate;
use tsi_rust::qtty;
use tsi_rust::services::ScheduleImportAdapter;
use tsi_rust::services::visibility_service::{VisibilityInput, compute_block_visibility};
use tsi_rust::siderust::bodies::Sun;
use tsi_rust::siderust::bodies::solar_system::Moon;
use tsi_rust::siderust::calculus::solar::Twilight;
use tsi_rust::siderust::time::intersect_periods;
use tsi_rust::siderust::{SearchOpts, altitude_ranges, below_threshold};

const FALLBACK_SCHEDULE_START_MJD: f64 = 60000.0;
const FALLBACK_SCHEDULE_END_MJD: f64 = 60007.0;
const DEFAULT_SCHEDULE_NAME: &str = "PhD Scheduling Problem";

#[derive(Debug, Clone)]
struct ResourceObservabilityConstraints {
    twilight: Twilight,
    moon_altitude_deg: Option<(f64, f64)>,
}

impl Default for ResourceObservabilityConstraints {
    fn default() -> Self {
        Self {
            twilight: Twilight::Astronomical,
            moon_altitude_deg: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct PhdScheduleImportAdapter;

impl ScheduleImportAdapter for PhdScheduleImportAdapter {
    fn name(&self) -> &'static str {
        "phd-scheduling-problem"
    }

    fn validate_schedule_payload(&self, raw_payload: &str) -> anyhow::Result<()> {
        let input: PhdSchedulingProblemRepr = serde_json::from_str(raw_payload)
            .context("Invalid PhD scheduling_problem JSON payload")?;
        validate_problem_shape(&input)
    }

    fn parse_schedule(&self, raw_payload: &str) -> anyhow::Result<Schedule> {
        let input: PhdSchedulingProblemRepr = serde_json::from_str(raw_payload)
            .context("Failed to parse PhD scheduling_problem payload")?;

        validate_problem_shape(&input)?;

        let (geographic_location, schedule_name, resource_constraints) =
            resolve_location_and_name(&input)?;
        let mut blocks = map_blocks(&input.scheduling_blocks)?;
        let schedule_period =
            resolve_schedule_period(input.schedule_time_window.as_ref(), &blocks)?;

        let astronomical_nights =
            tsi_rust::services::astronomical_night::compute_astronomical_nights(
                &geographic_location,
                &schedule_period,
            );

        let dark_periods = compute_dark_periods(
            &geographic_location,
            &schedule_period,
            &resource_constraints,
        )?;

        blocks.par_iter_mut().for_each(|block| {
            block.visibility_periods = compute_block_visibility(&VisibilityInput {
                location: &geographic_location,
                schedule_period: &schedule_period,
                target_ra: block.target_ra,
                target_dec: block.target_dec,
                constraints: &block.constraints,
                min_duration: block.min_observation,
                astronomical_nights: Some(&dark_periods),
            });
        });

        Ok(Schedule {
            id: None,
            name: schedule_name,
            checksum: tsi_rust::models::schedule::compute_schedule_checksum(raw_payload),
            schedule_period,
            dark_periods,
            geographic_location,
            astronomical_nights,
            blocks,
        })
    }
}

fn compute_dark_periods(
    geographic_location: &api::GeographicLocation,
    schedule_period: &api::Period,
    constraints: &ResourceObservabilityConstraints,
) -> anyhow::Result<Vec<api::Period>> {
    let night_periods = below_threshold(
        &Sun,
        geographic_location,
        *schedule_period,
        constraints.twilight.into(),
        SearchOpts::default(),
    );

    match constraints.moon_altitude_deg {
        Some((min_deg, max_deg)) => {
            let moon_ok_periods = altitude_ranges(
                &Moon,
                geographic_location,
                *schedule_period,
                qtty::Degrees::new(min_deg),
                qtty::Degrees::new(max_deg),
                SearchOpts::default(),
            );
            Ok(intersect_periods(&night_periods, &moon_ok_periods))
        }
        None => Ok(night_periods),
    }
}

pub fn phd_schedule_import_adapter() -> Arc<dyn ScheduleImportAdapter> {
    Arc::new(PhdScheduleImportAdapter)
}

fn validate_problem_shape(input: &PhdSchedulingProblemRepr) -> anyhow::Result<()> {
    if input.scheduling_blocks.is_empty() {
        bail!("scheduling_blocks array is empty");
    }

    if input.resources.is_empty() && input.location.is_none() {
        bail!(
            "missing observing site: expected resources[0].location or legacy top-level location"
        );
    }

    for block in &input.scheduling_blocks {
        if block.tasks.is_empty() {
            bail!("block {} has no tasks", block.id);
        }

        for task in &block.tasks {
            if let PhdTaskEntry::Id(task_id) = task {
                bail!(
                    "block {} contains bare task id {}: adapter requires full task objects",
                    block.id,
                    task_id
                );
            }
        }
    }

    Ok(())
}

fn resolve_location_and_name(
    input: &PhdSchedulingProblemRepr,
) -> anyhow::Result<(
    api::GeographicLocation,
    String,
    ResourceObservabilityConstraints,
)> {
    if let Some(primary) = input.resources.first() {
        if input.resources.len() > 1 {
            bail!(
                "multi-resource schedules are not supported: payload contains {} resources, \
                 expected exactly 1 (TSI is a single-site analysis tool)",
                input.resources.len()
            );
        }

        let location = map_location(&primary.location)?;
        let constraints = primary
            .hard_constraints
            .as_ref()
            .map(map_resource_constraints)
            .transpose()?
            .unwrap_or_default();
        let name = primary
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| DEFAULT_SCHEDULE_NAME.to_string());

        return Ok((location, name, constraints));
    }

    let fallback_location = input
        .location
        .as_ref()
        .context("missing observing site location")?;

    Ok((
        map_location(fallback_location)?,
        DEFAULT_SCHEDULE_NAME.to_string(),
        ResourceObservabilityConstraints::default(),
    ))
}

fn map_resource_constraints(
    repr: &ResourceHardConstraintsRepr,
) -> anyhow::Result<ResourceObservabilityConstraints> {
    let twilight = repr
        .night_time
        .as_ref()
        .map(|night| Twilight::from(night.twilight))
        .unwrap_or(Twilight::Astronomical);

    let moon_altitude_deg = if let Some(moon) = &repr.moon_altitude {
        if !moon.min_deg.is_finite() || !moon.max_deg.is_finite() {
            bail!("resource moon_altitude bounds must be finite");
        }
        if moon.min_deg > moon.max_deg {
            bail!(
                "resource moon_altitude has invalid bounds: min_deg {} > max_deg {}",
                moon.min_deg,
                moon.max_deg
            );
        }
        Some((moon.min_deg, moon.max_deg))
    } else {
        None
    };

    Ok(ResourceObservabilityConstraints {
        twilight,
        moon_altitude_deg,
    })
}

fn map_location(repr: &LocationRepr) -> anyhow::Result<api::GeographicLocation> {
    if !repr.longitude_deg.is_finite()
        || !repr.latitude_deg.is_finite()
        || !repr.height_m.is_finite()
    {
        bail!("location coordinates must be finite numbers");
    }

    Ok(api::GeographicLocation::new(
        qtty::Degrees::new(repr.longitude_deg),
        qtty::Degrees::new(repr.latitude_deg),
        qtty::Meters::new(repr.height_m),
    ))
}

fn map_blocks(blocks: &[PhdSchedulingBlockRepr]) -> anyhow::Result<Vec<api::SchedulingBlock>> {
    let mut mapped = Vec::new();

    for block in blocks {
        for task_entry in &block.tasks {
            let task = match task_entry {
                PhdTaskEntry::Task(task) => task,
                PhdTaskEntry::Id(task_id) => {
                    bail!(
                        "block {} contains bare task id {}: adapter requires full task objects",
                        block.id,
                        task_id
                    )
                }
            };

            mapped.push(map_task(block.id, task)?);
        }
    }

    if mapped.is_empty() {
        bail!("no schedulable tasks found in payload");
    }

    Ok(mapped)
}

fn map_task(parent_block_id: u64, task: &PhdTaskRepr) -> anyhow::Result<api::SchedulingBlock> {
    if !task.requested_duration_sec.is_finite() || task.requested_duration_sec <= 0.0 {
        bail!(
            "task {} has invalid requested_duration_sec {}",
            task.id,
            task.requested_duration_sec
        );
    }

    let target = task
        .target
        .as_ref()
        .with_context(|| format!("task {} is missing target coordinates", task.id))?;

    if !target.ra_deg.is_finite() || !target.dec_deg.is_finite() {
        bail!("task {} target coordinates must be finite", task.id);
    }

    let min_alt = task.hard_constraints.altitude_min_deg.unwrap_or(0.0);
    let max_alt = task.hard_constraints.altitude_max_deg.unwrap_or(90.0);
    let min_az = task.hard_constraints.azimuth_min_deg.unwrap_or(0.0);
    let max_az = task.hard_constraints.azimuth_max_deg.unwrap_or(360.0);

    if !min_alt.is_finite() || !max_alt.is_finite() || !min_az.is_finite() || !max_az.is_finite() {
        bail!("task {} has non-finite angular constraints", task.id);
    }

    let fixed_time = task
        .hard_constraints
        .time_window
        .as_ref()
        .map(map_time_window)
        .transpose()?;

    let scheduled_period = map_scheduled_period(task)?;

    let priority = task
        .soft_constraints
        .as_ref()
        .and_then(|soft| soft.priority)
        .unwrap_or(0.0);

    if !priority.is_finite() {
        bail!("task {} has non-finite priority", task.id);
    }

    let block_name = task
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("task-{}", task.id));

    Ok(api::SchedulingBlock {
        id: None,
        original_block_id: format!("{}:{}", parent_block_id, task.id),
        block_name,
        target_ra: qtty::Degrees::new(target.ra_deg),
        target_dec: qtty::Degrees::new(target.dec_deg),
        constraints: api::Constraints {
            min_alt: qtty::Degrees::new(min_alt),
            max_alt: qtty::Degrees::new(max_alt),
            min_az: qtty::Degrees::new(min_az),
            max_az: qtty::Degrees::new(max_az),
            fixed_time,
        },
        priority,
        min_observation: qtty::Seconds::new(task.requested_duration_sec),
        requested_duration: qtty::Seconds::new(task.requested_duration_sec),
        visibility_periods: Vec::new(),
        scheduled_period,
    })
}

fn map_scheduled_period(task: &PhdTaskRepr) -> anyhow::Result<Option<api::Period>> {
    if matches!(task.scheduled, Some(false)) {
        return Ok(None);
    }

    match (task.scheduled_start_mjd_utc, task.scheduled_end_mjd_utc) {
        (Some(start), Some(end)) => {
            if !start.is_finite() || !end.is_finite() {
                bail!("task {} has non-finite scheduled time bounds", task.id);
            }
            if start >= end {
                bail!(
                    "task {} has invalid scheduled window: start {} must be < end {}",
                    task.id,
                    start,
                    end
                );
            }

            Ok(Some(api::Period {
                start: ModifiedJulianDate::new(start),
                end: ModifiedJulianDate::new(end),
            }))
        }
        (None, None) => Ok(None),
        _ => bail!(
            "task {} has incomplete scheduled window: both scheduled_start_mjd_utc and scheduled_end_mjd_utc are required",
            task.id
        ),
    }
}

fn resolve_schedule_period(
    explicit_window: Option<&TimeWindowRepr>,
    blocks: &[api::SchedulingBlock],
) -> anyhow::Result<api::Period> {
    if let Some(window) = explicit_window {
        return map_time_window(window);
    }

    let mut min_start: Option<f64> = None;
    let mut max_end: Option<f64> = None;

    for block in blocks {
        if let Some(fixed_time) = &block.constraints.fixed_time {
            let start = fixed_time.start.value();
            let end = fixed_time.end.value();
            min_start = Some(min_start.map_or(start, |current| current.min(start)));
            max_end = Some(max_end.map_or(end, |current| current.max(end)));
        }
    }

    let start = min_start.unwrap_or(FALLBACK_SCHEDULE_START_MJD);
    let end = max_end.unwrap_or(FALLBACK_SCHEDULE_END_MJD);

    Ok(api::Period {
        start: ModifiedJulianDate::new(start),
        end: ModifiedJulianDate::new(end),
    })
}

fn map_time_window(window: &TimeWindowRepr) -> anyhow::Result<api::Period> {
    if !window.start_mjd_utc.is_finite() || !window.end_mjd_utc.is_finite() {
        bail!("time windows must use finite MJD values");
    }

    if window.start_mjd_utc >= window.end_mjd_utc {
        bail!(
            "invalid time window: start_mjd_utc {} must be < end_mjd_utc {}",
            window.start_mjd_utc,
            window.end_mjd_utc
        );
    }

    Ok(api::Period {
        start: ModifiedJulianDate::new(window.start_mjd_utc),
        end: ModifiedJulianDate::new(window.end_mjd_utc),
    })
}

#[derive(Debug, Deserialize)]
struct PhdSchedulingProblemRepr {
    #[serde(default)]
    resources: Vec<ResourceRepr>,
    #[serde(default)]
    location: Option<LocationRepr>,
    #[serde(default)]
    schedule_time_window: Option<TimeWindowRepr>,
    scheduling_blocks: Vec<PhdSchedulingBlockRepr>,
}

#[derive(Debug, Deserialize)]
struct ResourceRepr {
    #[serde(default)]
    name: Option<String>,
    location: LocationRepr,
    #[serde(default)]
    hard_constraints: Option<ResourceHardConstraintsRepr>,
}

#[derive(Debug, Deserialize)]
struct ResourceHardConstraintsRepr {
    #[serde(default)]
    night_time: Option<NightTimeConstraintRepr>,
    #[serde(default)]
    moon_altitude: Option<MoonAltitudeConstraintRepr>,
}

#[derive(Debug, Deserialize)]
struct NightTimeConstraintRepr {
    twilight: TwilightRepr,
}

#[derive(Debug, Deserialize)]
struct MoonAltitudeConstraintRepr {
    min_deg: f64,
    max_deg: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
enum TwilightRepr {
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

#[derive(Debug, Deserialize)]
struct LocationRepr {
    longitude_deg: f64,
    latitude_deg: f64,
    height_m: f64,
}

#[derive(Debug, Deserialize)]
struct PhdSchedulingBlockRepr {
    id: u64,
    tasks: Vec<PhdTaskEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PhdTaskEntry {
    Id(u64),
    Task(PhdTaskRepr),
}

#[derive(Debug, Deserialize)]
struct PhdTaskRepr {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    requested_duration_sec: f64,
    #[serde(default)]
    scheduled: Option<bool>,
    #[serde(default)]
    scheduled_start_mjd_utc: Option<f64>,
    #[serde(default)]
    scheduled_end_mjd_utc: Option<f64>,
    #[serde(default)]
    target: Option<TargetRepr>,
    #[serde(default)]
    hard_constraints: HardConstraintsRepr,
    #[serde(default)]
    soft_constraints: Option<SoftConstraintsRepr>,
}

#[derive(Debug, Deserialize)]
struct TargetRepr {
    ra_deg: f64,
    dec_deg: f64,
}

#[derive(Debug, Default, Deserialize)]
struct HardConstraintsRepr {
    #[serde(default)]
    altitude_min_deg: Option<f64>,
    #[serde(default)]
    altitude_max_deg: Option<f64>,
    #[serde(default)]
    azimuth_min_deg: Option<f64>,
    #[serde(default)]
    azimuth_max_deg: Option<f64>,
    #[serde(default)]
    time_window: Option<TimeWindowRepr>,
}

#[derive(Debug, Deserialize)]
struct SoftConstraintsRepr {
    #[serde(default)]
    priority: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct TimeWindowRepr {
    start_mjd_utc: f64,
    end_mjd_utc: f64,
}

#[cfg(test)]
mod tests {
    use super::{PhdScheduleImportAdapter, ScheduleImportAdapter};

    #[test]
    fn parses_minimal_phd_payload() {
        let payload = r#"{
            "resources": [{
                "name": "CTA-N",
                "location": {
                    "longitude_deg": -17.8925,
                    "latitude_deg": 28.7543,
                    "height_m": 2396.0
                }
            }],
            "schedule_time_window": {
                "start_mjd_utc": 61771.0,
                "end_mjd_utc": 61772.0
            },
            "scheduling_blocks": [{
                "id": 100,
                "tasks": [{
                    "id": 101,
                    "name": "test-task",
                    "requested_duration_sec": 1200.0,
                    "target": { "ra_deg": 83.63, "dec_deg": 22.01 },
                    "hard_constraints": {
                        "altitude_min_deg": 30.0,
                        "altitude_max_deg": 90.0,
                        "azimuth_min_deg": 0.0,
                        "azimuth_max_deg": 360.0,
                        "time_window": {
                            "start_mjd_utc": 61771.1,
                            "end_mjd_utc": 61771.2
                        }
                    },
                    "soft_constraints": { "priority": 3.0 }
                }],
                "dependencies": []
            }]
        }"#;

        let adapter = PhdScheduleImportAdapter;
        let schedule = adapter
            .parse_schedule(payload)
            .expect("payload should parse");

        assert_eq!(schedule.blocks.len(), 1);
        assert_eq!(schedule.blocks[0].original_block_id, "100:101");
        assert_eq!(schedule.blocks[0].block_name, "test-task");
        assert_eq!(schedule.name, "CTA-N");
    }

    #[test]
    fn maps_scheduled_window_when_present() {
        let payload = r#"{
            "resources": [{
                "name": "CTA-N",
                "location": {
                    "longitude_deg": -17.8925,
                    "latitude_deg": 28.7543,
                    "height_m": 2396.0
                }
            }],
            "schedule_time_window": {
                "start_mjd_utc": 61771.0,
                "end_mjd_utc": 61772.0
            },
            "scheduling_blocks": [{
                "id": 100,
                "tasks": [{
                    "id": 101,
                    "name": "scheduled-task",
                    "requested_duration_sec": 1200.0,
                    "scheduled": true,
                    "scheduled_start_mjd_utc": 61771.25,
                    "scheduled_end_mjd_utc": 61771.30,
                    "target": { "ra_deg": 83.63, "dec_deg": 22.01 },
                    "hard_constraints": {
                        "altitude_min_deg": 30.0,
                        "altitude_max_deg": 90.0,
                        "azimuth_min_deg": 0.0,
                        "azimuth_max_deg": 360.0,
                        "time_window": {
                            "start_mjd_utc": 61771.1,
                            "end_mjd_utc": 61771.4
                        }
                    },
                    "soft_constraints": { "priority": 3.0 }
                }],
                "dependencies": []
            }]
        }"#;

        let adapter = PhdScheduleImportAdapter;
        let schedule = adapter
            .parse_schedule(payload)
            .expect("payload should parse");

        let scheduled = schedule.blocks[0]
            .scheduled_period
            .expect("scheduled period should be mapped");
        assert_eq!(scheduled.start.value(), 61771.25);
        assert_eq!(scheduled.end.value(), 61771.30);
    }

    #[test]
    fn rejects_bare_task_ids() {
        let payload = r#"{
            "resources": [{
                "location": {
                    "longitude_deg": -17.8925,
                    "latitude_deg": 28.7543,
                    "height_m": 2396.0
                }
            }],
            "scheduling_blocks": [{
                "id": 1,
                "tasks": [2],
                "dependencies": []
            }]
        }"#;

        let adapter = PhdScheduleImportAdapter;
        let err = adapter
            .validate_schedule_payload(payload)
            .expect_err("payload with bare task IDs must fail");

        assert!(
            err.to_string()
                .contains("adapter requires full task objects"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resource_twilight_constraint_changes_dark_periods() {
        let payload = r#"{
            "resources": [{
                "name": "CTA-N",
                "location": {
                    "longitude_deg": -17.8925,
                    "latitude_deg": 28.7543,
                    "height_m": 2396.0
                },
                "hard_constraints": {
                    "night_time": { "twilight": "Civil" }
                }
            }],
            "schedule_time_window": {
                "start_mjd_utc": 61771.0,
                "end_mjd_utc": 61772.0
            },
            "scheduling_blocks": [{
                "id": 100,
                "tasks": [{
                    "id": 101,
                    "name": "test-task",
                    "requested_duration_sec": 1200.0,
                    "target": { "ra_deg": 83.63, "dec_deg": 22.01 },
                    "hard_constraints": {
                        "altitude_min_deg": 30.0,
                        "altitude_max_deg": 90.0,
                        "azimuth_min_deg": 0.0,
                        "azimuth_max_deg": 360.0,
                        "time_window": {
                            "start_mjd_utc": 61771.1,
                            "end_mjd_utc": 61771.2
                        }
                    },
                    "soft_constraints": { "priority": 3.0 }
                }],
                "dependencies": []
            }]
        }"#;

        let adapter = PhdScheduleImportAdapter;
        let schedule = adapter
            .parse_schedule(payload)
            .expect("payload should parse");

        let dark_hours: f64 = schedule
            .dark_periods
            .iter()
            .map(|p| (p.end.value() - p.start.value()) * 24.0)
            .sum();
        let astro_hours: f64 = schedule
            .astronomical_nights
            .iter()
            .map(|p| (p.end.value() - p.start.value()) * 24.0)
            .sum();

        assert!(
            dark_hours > astro_hours,
            "civil twilight dark-period coverage should be greater than astronomical-night coverage"
        );
    }
}
