//! CTAO JSON conversion logic.

use chrono::{DateTime, NaiveDate, Utc};
use schedulers::time::{MJD, Time};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::model::{
    CtaoFile, OutBlock, OutHardConstraints, OutSoftConstraints, OutTarget, OutTask, OutTimeWindow,
};

const DEFAULT_SCHEDULE_START_YEAR_UTC: i32 = 2028;
const DEFAULT_SCHEDULE_END_YEAR_UTC: i32 = 2029;

pub(crate) fn resolve_dataset_dir(arg: &str) -> PathBuf {
    let path = PathBuf::from(arg);
    if path.is_dir() {
        return path;
    }
    let candidates = [
        PathBuf::from("data").join(arg),
        PathBuf::from("../data").join(arg),
    ];
    for candidate in &candidates {
        if candidate.is_dir() {
            return candidate.clone();
        }
    }
    path
}

fn get_f64(v: &Value, key: &str) -> Option<f64> {
    v.get(key)?.as_f64()
}

fn utc_midnight(year: i32, month: u32, day: u32) -> Result<DateTime<Utc>, String> {
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| format!("invalid UTC date: {year:04}-{month:02}-{day:02}"))?;
    let datetime = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| format!("invalid UTC time: {year:04}-{month:02}-{day:02}T00:00:00Z"))?;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(datetime, Utc))
}

fn default_schedule_time_window() -> Result<OutTimeWindow, String> {
    let start_utc = utc_midnight(DEFAULT_SCHEDULE_START_YEAR_UTC, 1, 1)?;
    let end_utc = utc_midnight(DEFAULT_SCHEDULE_END_YEAR_UTC, 1, 1)?;

    Ok(OutTimeWindow {
        start_mjd_utc: Time::<MJD>::from_utc(start_utc).value(),
        end_mjd_utc: Time::<MJD>::from_utc(end_utc).value(),
    })
}

pub(crate) fn derive_schedule_time_window(blocks: &[OutBlock]) -> Result<OutTimeWindow, String> {
    let mut start_mjd_utc = f64::INFINITY;
    let mut end_mjd_utc = f64::NEG_INFINITY;

    for block in blocks {
        for task in &block.tasks {
            let Some(time_window) = task.hard_constraints.time_window.as_ref() else {
                continue;
            };
            start_mjd_utc = start_mjd_utc.min(time_window.start_mjd_utc);
            end_mjd_utc = end_mjd_utc.max(time_window.end_mjd_utc);
        }
    }

    if start_mjd_utc.is_finite() && end_mjd_utc.is_finite() {
        return Ok(OutTimeWindow {
            start_mjd_utc,
            end_mjd_utc,
        });
    }

    default_schedule_time_window()
}

fn target_name(target_obj: &Value) -> String {
    target_obj
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            target_obj
                .pointer("/science_targets/0/name")
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .to_string()
}

fn target_coordinate(target_obj: &Value) -> Option<&Value> {
    [
        "/position_/coord",
        "/position_/coordinate",
        "/science_targets/0/position_/coord",
        "/science_targets/0/position_/coordinate",
    ]
    .into_iter()
    .find_map(|pointer| target_obj.pointer(pointer))
}

pub(crate) fn convert_block(raw: &Value) -> Result<OutBlock, String> {
    let id = raw
        .get("scheduling_block_id")
        .and_then(Value::as_u64)
        .ok_or("missing scheduling_block_id")?;

    let target_obj = raw.get("target").ok_or("missing target")?;
    let name = target_name(target_obj);

    let coord = target_coordinate(target_obj).ok_or(
        "missing target coordinate (expected target.position_.coord or target.position_.coordinate)",
    )?;
    let ra_deg = get_f64(coord, "ra_in_deg").ok_or("missing ra_in_deg")?;
    let dec_deg = get_f64(coord, "dec_in_deg").ok_or("missing dec_in_deg")?;

    let constraints_root = raw
        .pointer("/scheduling_block_configuration_/constraints_")
        .ok_or("missing constraints_")?;

    let tc = constraints_root
        .get("time_constraint_")
        .ok_or("missing time_constraint_")?;
    let duration = get_f64(tc, "requested_duration_sec").ok_or("missing requested_duration_sec")?;
    let priority = raw.get("priority").and_then(Value::as_f64);

    let time_window = {
        let starts = tc.get("fixed_start_time").and_then(Value::as_array);
        let stops = tc.get("fixed_stop_time").and_then(Value::as_array);
        match (starts, stops) {
            (Some(s), Some(e)) if !s.is_empty() && !e.is_empty() => {
                let start_mjd = s[0].get("value").and_then(Value::as_f64);
                let end_mjd = e[0].get("value").and_then(Value::as_f64);
                match (start_mjd, end_mjd) {
                    (Some(start_mjd_utc), Some(end_mjd_utc)) => Some(OutTimeWindow {
                        start_mjd_utc,
                        end_mjd_utc,
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    };

    let ec = constraints_root.get("elevation_constraint_");
    let altitude_min_deg = ec.and_then(|e| get_f64(e, "min_elevation_angle_in_deg"));
    let altitude_max_deg = ec.and_then(|e| get_f64(e, "max_elevation_angle_in_deg"));

    let ac = constraints_root.get("azimuth_constraint_");
    let azimuth_min_deg = ac.and_then(|a| get_f64(a, "min_azimuth_angle_in_deg"));
    let azimuth_max_deg = ac.and_then(|a| get_f64(a, "max_azimuth_angle_in_deg"));

    let task = OutTask {
        id,
        name,
        requested_duration_sec: duration,
        target: OutTarget { ra_deg, dec_deg },
        hard_constraints: OutHardConstraints {
            altitude_min_deg,
            altitude_max_deg,
            azimuth_min_deg,
            azimuth_max_deg,
            time_window,
        },
        soft_constraints: priority.map(|priority| OutSoftConstraints { priority }),
    };

    Ok(OutBlock {
        id,
        tasks: vec![task],
        dependencies: vec![],
    })
}

pub(crate) fn normalize_duplicate_block_ids(blocks: &mut [OutBlock]) -> usize {
    let mut seen = HashSet::new();
    let mut next_id = blocks
        .iter()
        .map(|block| block.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut renamed = 0;

    for block in blocks {
        let original_id = block.id;
        if seen.insert(original_id) {
            continue;
        }

        while !seen.insert(next_id) {
            next_id = next_id.saturating_add(1);
        }

        block.id = next_id;
        for task in &mut block.tasks {
            if task.id == original_id {
                task.id = next_id;
            }
        }

        renamed += 1;
        next_id = next_id.saturating_add(1);
    }

    renamed
}

pub(crate) fn process_file(path: &Path) -> Result<Vec<OutBlock>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let ctao: CtaoFile =
        serde_json::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e))?;

    ctao.scheduling_block
        .iter()
        .map(|raw| {
            convert_block(raw).map_err(|e| format!("{}: block error: {}", path.display(), e))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_block_with_target(target: Value, id: u64) -> Value {
        json!({
            "scheduling_block_id": id,
            "priority": 2.5,
            "target": target,
            "scheduling_block_configuration_": {
                "constraints_": {
                    "time_constraint_": {
                        "requested_duration_sec": 1800.0,
                        "fixed_start_time": [{ "value": 60466.0 }],
                        "fixed_stop_time": [{ "value": 60467.0 }]
                    },
                    "elevation_constraint_": {
                        "min_elevation_angle_in_deg": 38.0,
                        "max_elevation_angle_in_deg": 90.0
                    },
                    "azimuth_constraint_": {
                        "min_azimuth_angle_in_deg": 0.0,
                        "max_azimuth_angle_in_deg": 359.9999
                    }
                }
            }
        })
    }

    #[test]
    fn convert_block_accepts_coordinate_target_shape() {
        let raw = sample_block_with_target(
            json!({
                "name": "PG1553+113",
                "position_": {
                    "coordinate": {
                        "ra_in_deg": 238.93625,
                        "dec_in_deg": 11.1947222
                    }
                }
            }),
            1000000001,
        );

        let block = convert_block(&raw).expect("block should convert");
        let task = &block.tasks[0];
        assert_eq!(task.name, "PG1553+113");
        assert_eq!(task.target.ra_deg, 238.93625);
        assert_eq!(task.target.dec_deg, 11.1947222);
    }

    #[test]
    fn normalize_duplicate_block_ids_keeps_ids_unique() {
        let mut blocks = vec![
            convert_block(&sample_block_with_target(
                json!({
                    "name": "a",
                    "position_": { "coord": { "ra_in_deg": 1.0, "dec_in_deg": 2.0 } }
                }),
                42,
            ))
            .expect("first block"),
            convert_block(&sample_block_with_target(
                json!({
                    "name": "b",
                    "position_": { "coordinate": { "ra_in_deg": 3.0, "dec_in_deg": 4.0 } }
                }),
                42,
            ))
            .expect("second block"),
        ];

        let renamed = normalize_duplicate_block_ids(&mut blocks);

        assert_eq!(renamed, 1);
        assert_eq!(blocks[0].id, 42);
        assert_ne!(blocks[1].id, 42);
        assert_eq!(blocks[1].tasks[0].id, blocks[1].id);
    }

    #[test]
    fn derive_schedule_time_window_uses_block_windows() {
        let blocks = vec![
            convert_block(&sample_block_with_target(
                json!({
                    "name": "a",
                    "position_": { "coord": { "ra_in_deg": 1.0, "dec_in_deg": 2.0 } }
                }),
                1,
            ))
            .expect("first block"),
            convert_block(&json!({
                "scheduling_block_id": 2,
                "target": {
                    "name": "b",
                    "position_": { "coord": { "ra_in_deg": 3.0, "dec_in_deg": 4.0 } }
                },
                "scheduling_block_configuration_": {
                    "constraints_": {
                        "time_constraint_": {
                            "requested_duration_sec": 1800.0,
                            "fixed_start_time": [{ "value": 60470.0 }],
                            "fixed_stop_time": [{ "value": 60475.0 }]
                        }
                    }
                }
            }))
            .expect("second block"),
        ];

        let window = derive_schedule_time_window(&blocks).expect("time window");

        assert_eq!(window.start_mjd_utc, 60466.0);
        assert_eq!(window.end_mjd_utc, 60475.0);
    }
}
