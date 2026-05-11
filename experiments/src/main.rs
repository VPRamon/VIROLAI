//! `experiments` binary entry point.
//!
//! Provides a `clap`-based CLI for running parameter-sweep experiments against
//! the `scheduler` library.
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
//! Loads an experiment spec, resolves the Cartesian product of cells, and
//! executes them in parallel.  With `--resume` it skips cells already marked
//! `completed` in the existing run's `state.jsonl`.  With `--dry-run` it only
//! resolves cells and writes `experiment.json` without running any scheduler.
//!
//! ## `migrate`
//!
//! ```text
//! experiments migrate <old_run_dir> [--output <new_dir>]
//! ```
//!
//! Ports a run directory produced by the deprecated `est_experiment` binary
//! into the current layout.

use clap::{Parser, Subcommand};
use experiments::cell::resolve_cells;
use experiments::migrate;
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
    /// Run an experiment matrix (default command).
    Run(RunArgs),
    /// Port a legacy `est_experiment` run directory to the current layout.
    Migrate(MigrateArgs),
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
}

// ── `migrate` sub-command ─────────────────────────────────────────────────────

/// Arguments for `experiments migrate`.
#[derive(Parser, Debug)]
struct MigrateArgs {
    /// Path to the legacy `est_experiment` run directory to migrate.
    old_run_dir: PathBuf,

    /// Output directory for the migrated run. Defaults to
    /// `<old_run_dir>/migrated`.
    #[arg(long, short, value_name = "DIR")]
    output: Option<PathBuf>,
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
        Cmd::Migrate(args) => migrate_cmd(args),
    }
}

// ── `run` implementation ──────────────────────────────────────────────────────

fn run(args: RunArgs) -> Result<(), String> {
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

    let summary = runner::execute(&spec, &cells, &run_dir, resume)?;
    println!(
        "experiments run done: {} cells total, {} skipped (resume), {} completed, {} failed",
        summary.total, summary.already_done, summary.completed, summary.failed
    );
    println!("artifacts -> {}", summary.run_dir.display());
    if summary.failed > 0 {
        return Err(format!(
            "{} cell(s) failed; see {}",
            summary.failed,
            run_dir.join(output::STATE_FILE).display()
        ));
    }
    Ok(())
}

// ── `migrate` implementation ──────────────────────────────────────────────────

fn migrate_cmd(args: MigrateArgs) -> Result<(), String> {
    let summary = migrate::migrate(&args.old_run_dir, args.output.as_deref())?;
    println!(
        "migrated {} cells from {} -> {}",
        summary.cells_migrated,
        args.old_run_dir.display(),
        summary.run_dir.display()
    );
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
