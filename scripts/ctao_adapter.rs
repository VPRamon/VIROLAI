// Converts CTAO (Cherenkov Telescope Array Observatory) dataset JSON files
// into a single `scheduling_problem.json` that conforms to
// `schemas/scheduling_problem/scheduling_problem.schema.json`.
//
// Each CTAO block maps to exactly one scheduler block containing one task:
//
// | CTAO field                                               | Output field                    |
// |----------------------------------------------------------|---------------------------------|
// | `scheduling_block_id`                                    | `block.id` and `task.id`        |
// | `target.name`                                            | `task.name`                     |
// | `target.position_.coord.ra_in_deg`                       | `task.target.ra_deg`            |
// | `target.position_.coord.dec_in_deg`                      | `task.target.dec_deg`           |
// | `constraints_.time_constraint_.requested_duration_sec`   | `task.requested_duration_sec`   |
// | `constraints_.elevation_constraint_.min/max`             | `task.hard_constraints.altitude_*`   |
// | `constraints_.azimuth_constraint_.min/max`               | `task.hard_constraints.azimuth_*`    |
// | `constraints_.time_constraint_.fixed_start/stop_time`    | `task.hard_constraints.time_window`  |
// | `priority`                                               | `task.soft_constraints.priority` |
// | inferred observatory (`CTA-N` / `CTA-S`)                 | `resources[0].location` + `resources[0].hard_constraints` |
//
// # Usage
// ```text
// cargo run --bin ctao_adapter -- <dataset_dir> [output_json]
// ```
// `<dataset_dir>` can be an absolute/relative path or one of the dataset
// short-names `CTA-N` / `CTA-S`, which are resolved to `data/<name>` relative
// to the workspace root.
//
// `[output_json]` defaults to `<dataset_dir>/scheduling_problem.json`.

use chrono::{DateTime, NaiveDate, Utc};
use scheduler::time::{MJD, Time};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use siderust::calculus::solar::Twilight;
use siderust::observatories::{EL_PARANAL, ROQUE_DE_LOS_MUCHACHOS};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_SCHEDULE_START_YEAR_UTC: i32 = 2028;
const DEFAULT_SCHEDULE_END_YEAR_UTC: i32 = 2029;
const DEFAULT_TELESCOPE_NIGHT_TWILIGHT: Twilight = Twilight::Nautical;
const DEFAULT_MOON_ALTITUDE_MIN_DEG: f64 = -90.0;
const DEFAULT_MOON_ALTITUDE_MAX_DEG: f64 = 0.0;

// ── output schema types ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct OutSchedulingProblem {
    resources: Vec<OutTelescope>,
    schedule_time_window: OutTimeWindow,
    scheduling_blocks: Vec<OutBlock>,
}

#[derive(Debug, Serialize)]
struct OutLocation {
    longitude_deg: f64,
    latitude_deg: f64,
    height_m: f64,
}

#[derive(Debug, Serialize)]
struct OutTelescope {
    id: u64,
    name: String,
    location: OutLocation,
    hard_constraints: OutResourceHardConstraints,
}

#[derive(Debug, Serialize)]
struct OutResourceHardConstraints {
    night_time: OutNightTimeConstraint,
    moon_altitude: OutMoonAltitudeConstraint,
}

#[derive(Debug, Serialize)]
struct OutNightTimeConstraint {
    twilight: String,
}

#[derive(Debug, Serialize)]
struct OutMoonAltitudeConstraint {
    min_deg: f64,
    max_deg: f64,
}

#[derive(Debug, Serialize)]
struct OutBlock {
    id: u64,
    tasks: Vec<OutTask>,
    dependencies: Vec<OutDependency>,
}

#[derive(Debug, Serialize)]
struct OutTask {
    id: u64,
    name: String,
    requested_duration_sec: f64,
    target: OutTarget,
    hard_constraints: OutHardConstraints,
    #[serde(skip_serializing_if = "Option::is_none")]
    soft_constraints: Option<OutSoftConstraints>,
}

#[derive(Debug, Serialize)]
struct OutTarget {
    ra_deg: f64,
    dec_deg: f64,
}

#[derive(Debug, Serialize)]
struct OutHardConstraints {
    #[serde(skip_serializing_if = "Option::is_none")]
    altitude_min_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    altitude_max_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azimuth_min_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    azimuth_max_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_window: Option<OutTimeWindow>,
}

#[derive(Debug, Serialize)]
struct OutSoftConstraints {
    priority: f64,
}

#[derive(Debug, Serialize)]
struct OutTimeWindow {
    start_mjd_utc: f64,
    end_mjd_utc: f64,
}

// An empty struct so the `dependencies` array is always `[]` for CTAO blocks
// (each block has exactly one task, so no intra-block dependencies exist).
#[derive(Debug, Serialize)]
struct OutDependency {}

// ── CTAO input types (minimal, untagged) ─────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CtaoFile {
    #[serde(rename = "SchedulingBlock")]
    scheduling_block: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CtaoObservatory {
    North,
    South,
}

impl CtaoObservatory {
    fn code(self) -> &'static str {
        match self {
            CtaoObservatory::North => "CTA-N",
            CtaoObservatory::South => "CTA-S",
        }
    }

    fn telescope_id(self) -> u64 {
        match self {
            CtaoObservatory::North => 0,
            CtaoObservatory::South => 1,
        }
    }

    fn telescope_resource(self) -> OutTelescope {
        let site = match self {
            CtaoObservatory::North => ROQUE_DE_LOS_MUCHACHOS,
            CtaoObservatory::South => EL_PARANAL,
        };

        OutTelescope {
            id: self.telescope_id(),
            name: self.code().to_string(),
            location: OutLocation {
                longitude_deg: site.lon.value(),
                latitude_deg: site.lat.value(),
                height_m: site.height.value(),
            },
            hard_constraints: OutResourceHardConstraints {
                night_time: OutNightTimeConstraint {
                    twilight: twilight_schema_name(DEFAULT_TELESCOPE_NIGHT_TWILIGHT).to_string(),
                },
                moon_altitude: OutMoonAltitudeConstraint {
                    min_deg: DEFAULT_MOON_ALTITUDE_MIN_DEG,
                    max_deg: DEFAULT_MOON_ALTITUDE_MAX_DEG,
                },
            },
        }
    }
}

fn twilight_schema_name(twilight: Twilight) -> &'static str {
    match twilight {
        Twilight::Civil => "Civil",
        Twilight::Nautical => "Nautical",
        Twilight::Astronomical => "Astronomical",
        Twilight::Horizon => "Horizon",
        Twilight::ApparentHorizon => "ApparentHorizon",
    }
}

// ── conversion ────────────────────────────────────────────────────────────────

fn resolve_dataset_dir(arg: &str) -> PathBuf {
    let path = PathBuf::from(arg);
    if path.is_dir() {
        return path;
    }
    // Try resolving short names relative to the workspace data/ directory.
    // Walk up from the binary location to find the workspace root.
    let candidates = [
        PathBuf::from("data").join(arg),
        PathBuf::from("../data").join(arg),
    ];
    for candidate in &candidates {
        if candidate.is_dir() {
            return candidate.clone();
        }
    }
    // Fall back to the original argument; error will surface when reading.
    path
}

fn get_f64(v: &Value, key: &str) -> Option<f64> {
    v.get(key)?.as_f64()
}

fn infer_observatory(
    dataset_dir: &Path,
    json_files: &[PathBuf],
) -> Result<CtaoObservatory, String> {
    let mut saw_north = false;
    let mut saw_south = false;
    let mut saw_lst_cycle = false;

    let dir = dataset_dir.to_string_lossy().to_ascii_uppercase();
    if dir.contains("CTA-N") || dir.contains("CTA_N") {
        saw_north = true;
    }
    if dir.contains("CTA-S") || dir.contains("CTA_S") {
        saw_south = true;
    }
    if dir.contains("LST") {
        saw_lst_cycle = true;
    }

    for path in json_files {
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let name = file_name.to_ascii_uppercase();
        if name.contains("_N_") || name.contains("CTA-N") || name.contains("CTA_N") {
            saw_north = true;
        }
        if name.contains("_S_") || name.contains("CTA-S") || name.contains("CTA_S") {
            saw_south = true;
        }
        if name.contains("LST") {
            saw_lst_cycle = true;
        }
    }

    match (saw_north, saw_south) {
        (true, false) => Ok(CtaoObservatory::North),
        (false, true) => Ok(CtaoObservatory::South),
        (false, false) if saw_lst_cycle => Ok(CtaoObservatory::North),
        (true, true) => Err(format!(
            "cannot infer a single observatory from {}: both CTA-N and CTA-S markers were found",
            dataset_dir.display()
        )),
        (false, false) => Err(format!(
            "cannot infer observatory from {}: expected CTA-N/CTA-S in directory or file names",
            dataset_dir.display()
        )),
    }
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

fn derive_schedule_time_window(blocks: &[OutBlock]) -> Result<OutTimeWindow, String> {
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

fn convert_block(raw: &Value) -> Result<OutBlock, String> {
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

fn normalize_duplicate_block_ids(blocks: &mut [OutBlock]) -> usize {
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

fn process_file(path: &Path) -> Result<Vec<OutBlock>, String> {
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <dataset_dir> [output_json]", args[0]);
        std::process::exit(1);
    }

    let dataset_dir = resolve_dataset_dir(&args[1]);
    let output_path = if args.len() >= 3 {
        PathBuf::from(&args[2])
    } else {
        dataset_dir.join("scheduling_problem.json")
    };

    let entries = match fs::read_dir(&dataset_dir) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("Cannot read directory {}: {}", dataset_dir.display(), err);
            std::process::exit(1);
        }
    };

    let mut all_blocks: Vec<OutBlock> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    let mut json_files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .filter(|p| p != &output_path)
        .collect();
    json_files.sort();

    for path in &json_files {
        match process_file(path) {
            Ok(blocks) => {
                println!(
                    "  {} blocks from {}",
                    blocks.len(),
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                all_blocks.extend(blocks);
            }
            Err(e) => errors.push(e),
        }
    }

    if !errors.is_empty() {
        for e in &errors {
            eprintln!("Warning: {e}");
        }
    }

    if all_blocks.is_empty() {
        eprintln!("No blocks found in {}", dataset_dir.display());
        std::process::exit(1);
    }

    let renamed = normalize_duplicate_block_ids(&mut all_blocks);
    if renamed > 0 {
        eprintln!(
            "Renumbered {renamed} duplicate scheduling block IDs to keep block/task IDs unique"
        );
    }

    let observatory = infer_observatory(&dataset_dir, &json_files).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    let schedule_time_window = derive_schedule_time_window(&all_blocks).unwrap_or_else(|e| {
        eprintln!("failed to build schedule_time_window: {e}");
        std::process::exit(1);
    });

    let block_count = all_blocks.len();
    let problem = OutSchedulingProblem {
        resources: vec![observatory.telescope_resource()],
        schedule_time_window,
        scheduling_blocks: all_blocks,
    };

    let json = serde_json::to_string_pretty(&problem).expect("serialization failed");

    fs::write(&output_path, &json).unwrap_or_else(|e| {
        eprintln!("Cannot write {}: {}", output_path.display(), e);
        std::process::exit(1);
    });

    println!(
        "Wrote {} blocks for {} to {}",
        block_count,
        observatory.code(),
        output_path.display()
    );
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
