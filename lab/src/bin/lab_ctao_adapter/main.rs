// Converts CTAO dataset JSON files into a scheduler `scheduling_problem.json`.
//
// Usage:
// cargo run -p lab --bin lab-ctao-adapter -- <dataset_dir> [output_json]

mod convert;
mod model;
mod observatory;

use convert::{
    derive_schedule_time_window, normalize_duplicate_block_ids, process_file, resolve_dataset_dir,
};
use model::OutSchedulingProblem;
use observatory::infer_observatory;
use std::fs;
use std::path::PathBuf;

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

    let mut all_blocks = Vec::new();
    let mut errors = Vec::new();

    let mut json_files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
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
