//! `lab run` command implementation.

use clap::Parser;
use lab::cell::resolve_cells;
use lab::runner::{RunOptions, execute};
use lab::spec::ExperimentSpec;
use std::path::{Path, PathBuf};

/// Arguments for `lab run`.
#[derive(Parser, Debug)]
pub(crate) struct RunArgs {
    /// Path to the experiment spec JSON.
    #[arg(long, value_name = "FILE")]
    spec: PathBuf,

    /// Path to the SQLite registry file.
    /// Defaults to `.lab/runs.sqlite`.
    #[arg(long, value_name = "PATH")]
    run_db: Option<PathBuf>,

    /// Re-execute cells that already have a row in the registry and update
    /// their stored metrics and schedule JSON.
    #[arg(long = "override")]
    override_existing: bool,
}

pub(crate) fn run(args: RunArgs) -> Result<(), String> {
    let spec = load_spec(&args.spec)?;
    let cells = resolve_cells(&spec)?;
    let opts = RunOptions {
        run_db: args.run_db,
        override_existing: args.override_existing,
    };
    let summary = execute(&spec, &cells, opts)?;
    println!(
        "lab run done: {} total, {} skipped, {} overridden, {} completed, {} failed",
        summary.total, summary.skipped, summary.overridden, summary.completed, summary.failed
    );
    if summary.failed > 0 {
        return Err(format!("{} cell(s) failed", summary.failed));
    }
    Ok(())
}

/// Loads and deserializes an [`ExperimentSpec`] from `path`, resolving relative
/// `datasets[*].path` entries against the spec file's parent directory.
fn load_spec(path: &Path) -> Result<ExperimentSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read spec {}: {e}", path.display()))?;
    let mut spec: ExperimentSpec = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse spec {}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));

    for d in &mut spec.datasets {
        d.path = resolve_relative(base, &d.path);
    }
    if let Some(ref dir) = spec.output_dir {
        spec.output_dir = Some(if dir.is_absolute() {
            dir.clone()
        } else {
            base.join(dir)
        });
    }
    Ok(spec)
}

/// Resolves `path` relative to `base`, preferring the `base`-joined form when
/// it exists on disk.
fn resolve_relative(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let candidate = base.join(path);
    if candidate.exists() {
        return candidate;
    }
    if path.exists() {
        return path.to_path_buf();
    }
    candidate
}
