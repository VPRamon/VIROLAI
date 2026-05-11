//! `experiments` binary entry point.
//!
//! Provides a `clap`-based CLI for running parameter-sweep experiments
//! against the `scheduler` library.
//!
//! # Commands
//!
//! ## `run` (default)
//!
//! ```text
//! experiments run --spec <experiment.json>
//!                 [--resume <existing_run_dir>]
//!                 [--output-dir <dir>]
//!                 [--dry-run]
//! ```
//!
//! Loads an experiment spec, resolves the Cartesian product of cells,
//! and executes them in parallel.  With `--resume` it skips cells
//! already marked `completed` in the existing run's `state.jsonl`.
//! With `--dry-run` it only resolves cells and writes
//! `experiment.json` without running any scheduler.

use clap::{Parser, Subcommand};
use experiments::cell::resolve_cells;
use experiments::output;
use experiments::runner;
use experiments::spec::ExperimentSpec;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

// ── Top-level CLI ─────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "experiments",
    version,
    about = "Run parameter-sweep experiments against the PhD scheduling library"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run an experiment matrix.
    Run(RunArgs),
}

// ── `run` sub-command ─────────────────────────────────────────────────────────

/// Arguments for `experiments run`.
#[derive(Parser, Debug)]
struct RunArgs {
    /// Path to the experiment spec JSON.
    #[arg(long, value_name = "FILE")]
    spec: PathBuf,

    /// Resume an existing run directory, skipping already-completed cells.
    #[arg(long, value_name = "DIR")]
    resume: Option<PathBuf>,

    /// Override the output directory declared in the spec (only meaningful
    /// with `--dry-run`).
    #[arg(long, value_name = "DIR")]
    output_dir: Option<PathBuf>,

    /// Resolve cells and write `experiment.json` without executing any
    /// scheduler runs.
    #[arg(long)]
    dry_run: bool,

    /// Skip writing `state.jsonl` entirely; print per-cell progress to stderr
    /// instead.  Incompatible with `--resume`.
    #[arg(long)]
    no_state: bool,
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    env_logger::init();
    let cli = Cli::parse();
    match dispatch(cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("experiments: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Run(args) => run(args),
    }
}

// ── `run` implementation ──────────────────────────────────────────────────────

fn run(args: RunArgs) -> Result<(), String> {
    if args.no_state && args.resume.is_some() {
        return Err("--no-state and --resume are mutually exclusive".to_string());
    }

    let spec = load_spec(&args.spec)?;
    let cells = resolve_cells(&spec)?;

    if args.dry_run {
        let target = args
            .output_dir
            .as_deref()
            .unwrap_or(&spec.output_dir)
            .to_path_buf();
        let run_dir = output::create_run_dir(&target, &spec.name)?;
        output::write_manifest(&run_dir, &spec, &cells)?;
        println!(
            "[dry-run] resolved {} cells; manifest -> {}",
            cells.len(),
            run_dir.join(output::EXPERIMENT_FILE).display()
        );
        for c in &cells {
            println!("  {}", c.cell_id);
        }
        return Ok(());
    }

    let (run_dir, resume) = if let Some(existing) = args.resume.as_ref() {
        (existing.clone(), true)
    } else {
        let dir = output::create_run_dir(&spec.output_dir, &spec.name)?;
        output::write_manifest(&dir, &spec, &cells)?;
        (dir, false)
    };

    let summary = runner::execute(&spec, &cells, &run_dir, resume, args.no_state)?;
    println!(
        "experiments run done: {} cells total, {} skipped (resume), {} completed, {} failed",
        summary.total, summary.already_done, summary.completed, summary.failed
    );
    println!("artifacts -> {}", summary.run_dir.display());
    if summary.failed > 0 {
        if args.no_state {
            return Err(format!("{} cell(s) failed", summary.failed));
        }
        return Err(format!(
            "{} cell(s) failed; see {}",
            summary.failed,
            run_dir.join(output::STATE_FILE).display()
        ));
    }
    Ok(())
}

// ── Spec loading ──────────────────────────────────────────────────────────────

/// Loads and deserialises an [`ExperimentSpec`] from `path`, resolving
/// relative `datasets[*].path` and `output_dir` entries against the spec
/// file's parent directory.
fn load_spec(path: &Path) -> Result<ExperimentSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read spec {}: {e}", path.display()))?;
    let mut spec: ExperimentSpec = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse spec {}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));

    for d in &mut spec.datasets {
        d.path = resolve_relative(base, &d.path);
    }
    spec.output_dir = if spec.output_dir.is_absolute() {
        spec.output_dir.clone()
    } else {
        base.join(&spec.output_dir)
    };
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
