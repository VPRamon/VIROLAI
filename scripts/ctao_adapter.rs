// Converts CTAO (Cherenkov Telescope Array Observatory) dataset JSON files
// into a single `scheduling_blocks.json` that conforms to
// `schemas/scheduling_blocks.schema.json`.
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
//
// # Usage
// ```text
// cargo run --bin ctao_adapter -- <dataset_dir> [output_json]
// ```
// `<dataset_dir>` can be an absolute/relative path or one of the dataset
// short-names `CTA-N` / `CTA-S`, which are resolved to `data/<name>` relative
// to the workspace root.
//
// `[output_json]` defaults to `<dataset_dir>/scheduling_blocks.json`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

// ── output schema types ───────────────────────────────────────────────────────

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

fn convert_block(raw: &Value) -> Result<OutBlock, String> {
    let id = raw
        .get("scheduling_block_id")
        .and_then(Value::as_u64)
        .ok_or("missing scheduling_block_id")?;

    let target_obj = raw.get("target").ok_or("missing target")?;
    let name = target_obj
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let coord = target_obj
        .pointer("/position_/coord")
        .ok_or("missing target.position_.coord")?;
    let ra_deg = get_f64(coord, "ra_in_deg").ok_or("missing ra_in_deg")?;
    let dec_deg = get_f64(coord, "dec_in_deg").ok_or("missing dec_in_deg")?;

    let constraints_root = raw
        .pointer("/scheduling_block_configuration_/constraints_")
        .ok_or("missing constraints_")?;

    let tc = constraints_root
        .get("time_constraint_")
        .ok_or("missing time_constraint_")?;
    let duration = get_f64(tc, "requested_duration_sec")
        .ok_or("missing requested_duration_sec")?;
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

fn process_file(path: &Path) -> Result<Vec<OutBlock>, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    let ctao: CtaoFile = serde_json::from_str(&text)
        .map_err(|e| format!("{}: {}", path.display(), e))?;

    ctao.scheduling_block
        .iter()
        .map(|raw| {
            convert_block(raw)
                .map_err(|e| format!("{}: block error: {}", path.display(), e))
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "Usage: {} <dataset_dir> [output_json]",
            args[0]
        );
        std::process::exit(1);
    }

    let dataset_dir = resolve_dataset_dir(&args[1]);
    let output_path = if args.len() >= 3 {
        PathBuf::from(&args[2])
    } else {
        dataset_dir.join("scheduling_blocks.json")
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

    let json = serde_json::to_string_pretty(&all_blocks)
        .expect("serialization failed");

    fs::write(&output_path, &json).unwrap_or_else(|e| {
        eprintln!("Cannot write {}: {}", output_path.display(), e);
        std::process::exit(1);
    });

    println!(
        "Wrote {} blocks to {}",
        all_blocks.len(),
        output_path.display()
    );
}
