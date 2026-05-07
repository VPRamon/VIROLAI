//! `experiment_matrix` binary entry point.

mod cell;
mod migrate;
mod output;
mod runner;
mod spec;
mod state;

// Reuse the `est_experiment` binary's `config` and `problem` modules
// in-place so there's a single source of truth for sweep axes and
// dataset preparation.
mod est_experiment;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cell::resolve_cells;
use crate::spec::ExperimentSpec;

fn main() -> ExitCode {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e == "help requested" => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let program = args
        .first()
        .cloned()
        .unwrap_or_else(|| "experiment_matrix".to_string());
    let rest = &args[1..];

    if rest.first().map(String::as_str) == Some("migrate") {
        return run_migrate(&program, &rest[1..]);
    }

    let cli = parse_cli(&program, rest)?;
    let spec_path = cli
        .spec_path
        .ok_or_else(|| "missing --spec <path>; pass `--help` for usage".to_string())?;
    let spec = load_spec(&spec_path)?;
    let cells = resolve_cells(&spec)?;

    if cli.dry_run {
        let target = if let Some(out) = cli.output_dir_override.as_ref() {
            out.clone()
        } else {
            spec.output_dir.clone()
        };
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

    let (run_dir, resume) = if let Some(existing) = cli.resume_dir.as_ref() {
        (existing.clone(), true)
    } else {
        let dir = output::create_run_dir(&spec.output_dir, &spec.name)?;
        output::write_manifest(&dir, &spec, &cells)?;
        (dir, false)
    };

    let summary = runner::execute(&spec, &cells, &run_dir, resume)?;
    println!(
        "experiment_matrix done: {} cells total, {} skipped (resume), {} completed, {} failed",
        summary.total, summary.already_done, summary.completed, summary.failed
    );
    println!("artifacts -> {}", summary.run_dir.display());
    if summary.failed > 0 {
        return Err(format!(
            "{} cell(s) failed; see state.jsonl",
            summary.failed
        ));
    }
    Ok(())
}

fn run_migrate(program: &str, args: &[String]) -> Result<(), String> {
    let mut old_dir: Option<PathBuf> = None;
    let mut output_dir: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                output_dir = Some(PathBuf::from(flag_arg(args, &mut i, "--output")?));
            }
            "-h" | "--help" => {
                print_migrate_usage(program);
                return Err("help requested".to_string());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown migrate flag '{other}'"));
            }
            other => {
                if old_dir.is_some() {
                    return Err(format!("unexpected positional '{other}'"));
                }
                old_dir = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    let old_dir = old_dir.ok_or_else(|| {
        print_migrate_usage(program);
        "missing <old_run_dir>".to_string()
    })?;
    let summary = migrate::migrate(&old_dir, output_dir.as_deref())?;
    println!(
        "migrated {} cells from {} -> {}",
        summary.cells_migrated,
        old_dir.display(),
        summary.run_dir.display()
    );
    Ok(())
}

fn load_spec(path: &Path) -> Result<ExperimentSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read spec {}: {e}", path.display()))?;
    let mut spec: ExperimentSpec = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse spec {}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    for d in &mut spec.datasets {
        d.path = resolve_relative(base, &d.path);
    }
    // output_dir is always treated as relative to the spec file (or used as-is
    // when absolute) — its existence is irrelevant.
    spec.output_dir = if spec.output_dir.is_absolute() {
        spec.output_dir.clone()
    } else {
        base.join(&spec.output_dir)
    };
    Ok(spec)
}

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

// ── CLI parsing ──────────────────────────────────────────────────────────────

#[derive(Default)]
struct CliArgs {
    spec_path: Option<PathBuf>,
    resume_dir: Option<PathBuf>,
    output_dir_override: Option<PathBuf>,
    dry_run: bool,
}

fn parse_cli(program: &str, args: &[String]) -> Result<CliArgs, String> {
    let mut cli = CliArgs::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--spec" => cli.spec_path = Some(PathBuf::from(flag_arg(args, &mut i, "--spec")?)),
            "--resume" => {
                cli.resume_dir = Some(PathBuf::from(flag_arg(args, &mut i, "--resume")?));
            }
            "--output-dir" => {
                cli.output_dir_override =
                    Some(PathBuf::from(flag_arg(args, &mut i, "--output-dir")?));
            }
            "--dry-run" => {
                cli.dry_run = true;
                i += 1;
            }
            "-h" | "--help" => {
                print_usage(program);
                return Err("help requested".to_string());
            }
            other => {
                print_usage(program);
                return Err(format!("unknown argument '{other}'"));
            }
        }
    }
    Ok(cli)
}

fn flag_arg<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    match args.get(*i + 1) {
        Some(v) => {
            *i += 2;
            Ok(v.as_str())
        }
        None => Err(format!("missing value for {flag}")),
    }
}

fn print_usage(program: &str) {
    eprintln!(
        "Usage: {program} --spec <path>\n\
         \x20      [--resume <existing_run_dir>]\n\
         \x20      [--output-dir <dir>]   override the spec's output_dir for --dry-run\n\
         \x20      [--dry-run]            only resolve cells and write experiment.json\n\
         \x20  {program} migrate <old_run_dir> [--output <new_dir>]\n"
    );
}

fn print_migrate_usage(program: &str) {
    eprintln!("Usage: {program} migrate <old_run_dir> [--output <new_dir>]");
}
