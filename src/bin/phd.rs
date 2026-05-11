//! `phd` — unified user-facing CLI for the PhD scheduling workspace.
//!
//! Phase 1 (foundation) responsibilities:
//! - `phd run` / `phd matrix` / `phd dataset adapt` — dispatch to the
//!   existing sibling binaries (`scheduler`, `experiments`,
//!   `ctao_adapter`) so users only need to remember one entry point.
//! - `phd manifest create` — walk a `run-<ts>/` directory produced by
//!   `experiments` and emit per-cell `manifest.json` files plus a
//!   batch index (`manifest-batch.json`) under
//!   `<run-dir>/cells/<cell_id>/manifest.json`.
//! - `phd manifest validate` — load a manifest and run the structural
//!   validator from [`scheduler::manifest`].
//! - `phd publish` — stub returning a clear "not yet implemented" error;
//!   wired in Phase 3.
//!
//! Subprocess dispatch uses the sibling binary that lives next to
//! `phd` in the same directory (the cargo `target/.../` layout). When
//! that lookup fails the CLI falls back to the binary name on `$PATH`
//! so users can install only what they need.

use clap::{Parser, Subcommand};
use scheduler::manifest::{
    AlgorithmRef, ArtifactRef, DatasetRef, Horizon, Manifest, Producer, Provenance, RunInfo,
    RunKind, RunStatus, ValidationReport, ValidationStatus,
};
use scheduler::metrics::ScheduleMetrics;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const PHD_VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_SHA: Option<&str> = option_env!("GIT_SHA");

#[derive(Parser, Debug)]
#[command(
    name = "phd",
    version,
    about = "PhD scheduling CLI — run simulations, build manifests, publish to the webapp",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run a single scheduling problem (delegates to the `scheduler` binary).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Run {
        /// Forwarded as-is to the `scheduler` binary.
        args: Vec<String>,
    },
    /// Run a sweep / matrix experiment (delegates to `experiments`).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Matrix {
        /// Forwarded as-is to the `experiments` binary.
        args: Vec<String>,
    },
    /// Run a sweep and collect flat results — a clean alternative to `phd matrix`.
    ///
    /// Executes the experiment defined in SPEC, writes one self-contained
    /// schedule JSON per cell to OUT (flat, no subdirectories), and optionally
    /// emits a companion `<cell_id>.manifest.json` next to each schedule.
    Sweep {
        /// Path to the experiment spec JSON (same format as `phd matrix --spec`).
        #[arg(long, value_name = "FILE")]
        spec: PathBuf,
        /// Output directory (created if absent). Schedule files land here flat.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Also write `<cell_id>.manifest.json` next to each schedule.
        #[arg(long)]
        manifest: bool,
        /// Override parallelism (threads). Defaults to spec's `max_parallel`.
        #[arg(long, value_name = "N")]
        parallel: Option<usize>,
    },
    /// Dataset utilities.
    Dataset {
        #[command(subcommand)]
        cmd: DatasetCmd,
    },
    /// Manifest operations (build / validate / inspect).
    Manifest {
        #[command(subcommand)]
        cmd: ManifestCmd,
    },
    /// Publish manifest(s) to a webapp workspace. (Phase 3 — not yet implemented.)
    Publish(PublishArgs),
}

#[derive(Subcommand, Debug)]
enum DatasetCmd {
    /// CTA-O dataset adapter (delegates to `ctao_adapter`).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Adapt { args: Vec<String> },
}

#[derive(Subcommand, Debug)]
enum ManifestCmd {
    /// Build manifest(s) from a `experiments` run directory or a single schedule JSON.
    ///
    /// Use `--run <DIR>` to build manifests for all cells in an experiment run, or
    /// `--schedule <FILE>` to build a manifest from a single self-contained schedule JSON.
    Create {
        /// Path to the `run-<ts>/` directory produced by `phd matrix` or `phd sweep`.
        /// Conflicts with `--schedule`.
        #[arg(long, value_name = "DIR", conflicts_with = "schedule")]
        run: Option<PathBuf>,
        /// Path to a single schedule JSON (must contain embedded `schedule_metadata` and
        /// `schedule_metrics`). Conflicts with `--run`.
        #[arg(long, value_name = "FILE", conflicts_with = "run")]
        schedule: Option<PathBuf>,
        /// Optional output path.
        ///  - With `--run`:      output directory (defaults to `<run>/cells`).
        ///  - With `--schedule`: output file (defaults to stdout).
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Skip cells whose manifest already exists (only applies with `--run`).
        #[arg(long)]
        skip_existing: bool,
    },
    /// Validate a manifest against the structural rules + schema invariants.
    Validate {
        /// Path to a `manifest.json` file.
        path: PathBuf,
    },
}

#[derive(Parser, Debug)]
struct PublishArgs {
    /// Target workspace id (slug). Created if `--create-workspace` is set.
    #[arg(long)]
    workspace: String,
    /// Publish a single manifest file.
    #[arg(long, value_name = "FILE", conflicts_with = "manifest_dir")]
    manifest: Option<PathBuf>,
    /// Publish every `manifest.json` found under DIR (recursive).
    #[arg(long = "manifest-dir", value_name = "DIR")]
    manifest_dir: Option<PathBuf>,
    /// Webapp base URL (default: http://localhost:8080 or $PHD_WEBAPP_URL).
    #[arg(long, value_name = "URL")]
    url: Option<String>,
    /// Optional bearer token (or set $PHD_WEBAPP_TOKEN).
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
    /// Create the workspace if it doesn't exist.
    #[arg(long = "create-workspace")]
    create_workspace: bool,
    /// Workspace display name when creating (defaults to id).
    #[arg(long = "workspace-name")]
    workspace_name: Option<String>,
    /// Maximum retry attempts for transient failures.
    #[arg(long, default_value_t = 3)]
    retries: u32,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli.cmd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("phd: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cmd: Cmd) -> Result<ExitCode, String> {
    match cmd {
        Cmd::Run { args } => exec_sibling("scheduler", &args),
        Cmd::Matrix { args } => exec_sibling("experiments", &args),
        Cmd::Sweep {
            spec,
            out,
            manifest,
            parallel,
        } => sweep(&spec, &out, manifest, parallel),
        Cmd::Dataset {
            cmd: DatasetCmd::Adapt { args },
        } => exec_sibling("ctao_adapter", &args),
        Cmd::Manifest {
            cmd:
                ManifestCmd::Create {
                    run,
                    schedule,
                    out,
                    skip_existing,
                },
        } => match (run, schedule) {
            (Some(run_dir), None) => manifest_create(&run_dir, out.as_deref(), skip_existing),
            (None, Some(sched_file)) => manifest_create_from_schedule(&sched_file, out.as_deref()),
            _ => Err("phd manifest create: specify exactly one of --run or --schedule".to_string()),
        },
        Cmd::Manifest {
            cmd: ManifestCmd::Validate { path },
        } => manifest_validate(&path),
        Cmd::Publish(args) => publish(args),
    }
}

// ---------------------------------------------------------------------------
// Sibling binary dispatch
// ---------------------------------------------------------------------------

fn exec_sibling(name: &str, args: &[String]) -> Result<ExitCode, String> {
    let exe = locate_sibling(name);
    let status = Command::new(&exe)
        .args(args)
        .status()
        .map_err(|e| format!("failed to spawn `{}`: {e}", exe.display()))?;
    Ok(if let Some(code) = status.code() {
        if code == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::from((code & 0xff) as u8)
        }
    } else {
        ExitCode::FAILURE
    })
}

fn locate_sibling(name: &str) -> PathBuf {
    if let Ok(current) = std::env::current_exe()
        && let Some(dir) = current.parent()
    {
        let candidate = dir.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        });
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from(name)
}

// ---------------------------------------------------------------------------
// `phd sweep`
// ---------------------------------------------------------------------------

fn sweep(
    spec_path: &Path,
    out_dir: &Path,
    emit_manifest: bool,
    parallel_override: Option<usize>,
) -> Result<ExitCode, String> {
    if !spec_path.is_file() {
        return Err(format!("spec file `{}` not found", spec_path.display()));
    }

    // Build a temp output dir for the experiments run.
    let tmp = tempfile::TempDir::new().map_err(|e| format!("failed to create temp dir: {e}"))?;
    let tmp_out = tmp.path().join("sweep_run");
    fs::create_dir_all(&tmp_out).map_err(|e| format!("failed to create temp output dir: {e}"))?;

    let spec_for_run: PathBuf =
        patch_spec_for_run(spec_path, tmp.path(), &tmp_out, parallel_override)?;

    // Run experiments.
    let status = Command::new(locate_sibling("experiments"))
        .arg("run")
        .arg("--spec")
        .arg(&spec_for_run)
        .status()
        .map_err(|e| format!("failed to spawn experiments: {e}"))?;
    if !status.success() {
        return Err("experiments run failed".to_string());
    }

    // Find the single run-<ts> directory experiments created.
    let run_dir = find_single_run_dir(&tmp_out)
        .ok_or_else(|| format!("could not locate run dir under {}", tmp_out.display()))?;

    // Ensure the flat output directory exists.
    fs::create_dir_all(out_dir)
        .map_err(|e| format!("failed to create output dir {}: {e}", out_dir.display()))?;

    // Read the experiment manifest for manifest generation.
    let exp_path = run_dir.join("experiment.json");
    let exp_text = fs::read_to_string(&exp_path)
        .map_err(|e| format!("failed to read experiment.json: {e}"))?;
    let exp: ExperimentManifestFile = serde_json::from_str(&exp_text)
        .map_err(|e| format!("failed to parse experiment.json: {e}"))?;
    let dataset_lookup: std::collections::HashMap<&str, &DatasetSpecLite> = exp
        .spec
        .datasets
        .iter()
        .map(|d| (d.id.as_str(), d))
        .collect();
    let now = current_rfc3339();
    let run_id = run_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("run")
        .to_string();

    // Flatten: copy schedules to out_dir, optionally write manifests.
    let schedules_dir = run_dir.join("schedules");
    let mut copied = 0usize;
    let mut manifests_written = 0usize;

    for cell in &exp.cells {
        let src = schedules_dir.join(format!("{}.json", cell.cell_id));
        if !src.exists() {
            eprintln!("phd sweep: warning: missing schedule for {}", cell.cell_id);
            continue;
        }
        let dst = out_dir.join(format!("{}.json", cell.cell_id));
        fs::copy(&src, &dst)
            .map_err(|e| format!("failed to copy {} -> {}: {e}", src.display(), dst.display()))?;
        copied += 1;

        if emit_manifest {
            match build_cell_manifest(
                &run_dir,
                &exp.spec.name,
                &run_id,
                &now,
                &dataset_lookup,
                cell,
            ) {
                Ok(m) => {
                    let manifest_path = out_dir.join(format!("{}.manifest.json", cell.cell_id));
                    let text = serde_json::to_string_pretty(&m)
                        .map_err(|e| format!("failed to serialize manifest: {e}"))?;
                    fs::write(&manifest_path, &text)
                        .map_err(|e| format!("failed to write {}: {e}", manifest_path.display()))?;
                    manifests_written += 1;
                }
                Err(e) => {
                    eprintln!(
                        "phd sweep: warning: manifest for `{}` failed: {e}",
                        cell.cell_id
                    );
                }
            }
        }
    }

    println!(
        "phd sweep: {} schedule(s) written to {}{}",
        copied,
        out_dir.display(),
        if emit_manifest {
            format!(" + {manifests_written} manifest(s)")
        } else {
            String::new()
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn patch_spec_for_run(
    spec_path: &Path,
    tmp_dir: &Path,
    tmp_out: &Path,
    parallel_override: Option<usize>,
) -> Result<PathBuf, String> {
    let text = fs::read_to_string(spec_path).map_err(|e| format!("failed to read spec: {e}"))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("failed to parse spec: {e}"))?;
    if let Some(obj) = v.as_object_mut() {
        if let Some(n) = parallel_override {
            obj.insert("max_parallel".to_string(), serde_json::json!(n));
        }
        obj.insert("output_dir".to_string(), serde_json::json!(tmp_out));
        let base = spec_path.parent().unwrap_or_else(|| Path::new("."));
        resolve_dataset_paths(obj, base);
    }

    let patched_path = tmp_dir.join("spec_patched.json");
    let mut f = fs::File::create(&patched_path)
        .map_err(|e| format!("failed to write patched spec: {e}"))?;
    let patched = serde_json::to_string_pretty(&v)
        .map_err(|e| format!("failed to serialize patched spec: {e}"))?;
    f.write_all(patched.as_bytes())
        .map_err(|e| format!("failed to write patched spec: {e}"))?;
    Ok(patched_path)
}

fn resolve_dataset_paths(obj: &mut serde_json::Map<String, serde_json::Value>, base: &Path) {
    if let Some(serde_json::Value::Array(datasets)) = obj.get_mut("datasets") {
        for dataset in datasets.iter_mut() {
            if let Some(dataset_obj) = dataset.as_object_mut()
                && let Some(serde_json::Value::String(path_str)) = dataset_obj.get_mut("path")
            {
                let path = Path::new(path_str);
                if path.is_absolute() {
                    continue;
                }
                let resolved = resolve_relative(base, path);
                *path_str = resolved.to_string_lossy().into_owned();
            }
        }
    }
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

/// Find the single `run-<ts>/` directory inside `<out>/<exp_slug>/`.
fn find_single_run_dir(tmp_out: &Path) -> Option<PathBuf> {
    // experiments creates <tmp_out>/<exp_slug>/run-<ts>/
    let exp_dirs: Vec<PathBuf> = fs::read_dir(tmp_out)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    for exp_dir in &exp_dirs {
        let run_dirs: Vec<PathBuf> = fs::read_dir(exp_dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        if let Some(run_dir) = run_dirs.into_iter().next() {
            return Some(run_dir);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// `phd manifest create`
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ExperimentManifestFile {
    spec: ExperimentSpecLite,
    cells: Vec<MatrixCellLite>,
}

#[derive(Debug, Deserialize)]
struct ExperimentSpecLite {
    name: String,
    #[serde(default)]
    datasets: Vec<DatasetSpecLite>,
}

#[derive(Debug, Deserialize)]
struct DatasetSpecLite {
    id: String,
    #[serde(default)]
    label: Option<String>,
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct MatrixCellLite {
    cell_id: String,
    dataset_id: String,
    dataset_path: PathBuf,
    #[serde(default)]
    dataset_label: Option<String>,
    algorithm: String,
    run_config: serde_json::Value,
}

fn manifest_create(
    run_dir: &Path,
    out_override: Option<&Path>,
    skip_existing: bool,
) -> Result<ExitCode, String> {
    if !run_dir.is_dir() {
        return Err(format!("run dir `{}` does not exist", run_dir.display()));
    }
    let exp_path = run_dir.join("experiment.json");
    let exp_text = fs::read_to_string(&exp_path)
        .map_err(|e| format!("failed to read {}: {e}", exp_path.display()))?;
    let exp: ExperimentManifestFile = serde_json::from_str(&exp_text)
        .map_err(|e| format!("failed to parse {}: {e}", exp_path.display()))?;

    let out_root = out_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| run_dir.join("cells"));
    fs::create_dir_all(&out_root)
        .map_err(|e| format!("failed to create {}: {e}", out_root.display()))?;

    let now = current_rfc3339();
    let run_id = run_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("run")
        .to_string();

    let dataset_lookup: std::collections::HashMap<&str, &DatasetSpecLite> = exp
        .spec
        .datasets
        .iter()
        .map(|d| (d.id.as_str(), d))
        .collect();

    let mut emitted: Vec<(String, PathBuf, String)> = Vec::new();
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for cell in &exp.cells {
        let cell_dir = out_root.join(&cell.cell_id);
        let manifest_path = cell_dir.join("manifest.json");
        if skip_existing && manifest_path.exists() {
            skipped += 1;
            continue;
        }

        match build_cell_manifest(
            run_dir,
            &exp.spec.name,
            &run_id,
            &now,
            &dataset_lookup,
            cell,
        ) {
            Ok(m) => {
                fs::create_dir_all(&cell_dir)
                    .map_err(|e| format!("failed to create {}: {e}", cell_dir.display()))?;
                let text = serde_json::to_string_pretty(&m)
                    .map_err(|e| format!("failed to serialize manifest: {e}"))?;
                fs::write(&manifest_path, &text)
                    .map_err(|e| format!("failed to write {}: {e}", manifest_path.display()))?;
                emitted.push((cell.cell_id.clone(), manifest_path, m.manifest_id));
            }
            Err(e) => {
                eprintln!("phd: warning: skipping cell `{}`: {e}", cell.cell_id);
                failed += 1;
            }
        }
    }

    // Batch index next to the experiment manifest.
    let batch_path = run_dir.join("manifest-batch.json");
    let batch = serde_json::json!({
        "schema": "phd-manifest-batch/1",
        "batch_id": uuid::Uuid::new_v4().to_string(),
        "run_id": run_id,
        "experiment": exp.spec.name,
        "created_at": now,
        "manifests": emitted
            .iter()
            .map(|(cid, path, mid)| serde_json::json!({
                "cell_id": cid,
                "manifest_id": mid,
                "path": path.strip_prefix(run_dir).unwrap_or(path).display().to_string(),
            }))
            .collect::<Vec<_>>(),
    });
    fs::write(
        &batch_path,
        serde_json::to_string_pretty(&batch)
            .map_err(|e| format!("failed to serialize batch index: {e}"))?,
    )
    .map_err(|e| format!("failed to write {}: {e}", batch_path.display()))?;

    println!(
        "phd manifest create: {} written, {} skipped, {} failed → {}",
        emitted.len(),
        skipped,
        failed,
        out_root.display()
    );
    println!("batch index → {}", batch_path.display());

    if failed > 0 && emitted.is_empty() {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

// ---------------------------------------------------------------------------
// `phd manifest create --schedule`
// ---------------------------------------------------------------------------

fn manifest_create_from_schedule(
    schedule_path: &Path,
    out: Option<&Path>,
) -> Result<ExitCode, String> {
    let text = fs::read_to_string(schedule_path)
        .map_err(|e| format!("failed to read {}: {e}", schedule_path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse {}: {e}", schedule_path.display()))?;

    // Extract schedule_metadata.
    let meta = doc
        .get("schedule_metadata")
        .ok_or_else(|| "schedule JSON has no `schedule_metadata` field — run with `phd sweep --manifest` or `phd matrix` first".to_string())?;

    let algorithm = meta
        .get("algorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let algorithm_config = meta
        .get("algorithm_config")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let dataset_id = meta
        .get("dataset_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let dataset_label = meta
        .get("dataset_label")
        .and_then(|v| v.as_str())
        .unwrap_or(&dataset_id)
        .to_string();

    let (start_mjd, end_mjd) = if let Some(period) = meta.get("period") {
        let s = period
            .get("start_mjd_utc")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let e = period
            .get("end_mjd_utc")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        (s, e)
    } else {
        (0.0, 0.0)
    };

    // Extract schedule_metrics.
    let metrics: ScheduleMetrics = if let Some(m) = doc.get("schedule_metrics") {
        serde_json::from_value(m.clone())
            .map_err(|e| format!("failed to parse `schedule_metrics`: {e}"))?
    } else {
        return Err("schedule JSON has no `schedule_metrics` field — rebuild schedule with `phd sweep` or `phd matrix` (recent version)".to_string());
    };

    let now = current_rfc3339();
    let schedule_sha = sha256_of(schedule_path).unwrap_or_else(|_| "0".repeat(64));
    let schedule_size = fs::metadata(schedule_path).map(|m| m.len()).unwrap_or(0);

    let manifest = Manifest {
        manifest_schema_version: scheduler::manifest::MANIFEST_SCHEMA_VERSION.to_string(),
        manifest_id: uuid::Uuid::new_v4().to_string(),
        created_at: now.clone(),
        producer: Producer {
            name: "phd".to_string(),
            version: PHD_VERSION.to_string(),
            git_sha: GIT_SHA.map(str::to_string),
            host: hostname(),
        },
        dataset: DatasetRef {
            id: dataset_id,
            name: dataset_label,
            source_path: String::new(),
            sha256: String::new(),
            schema_version: "scheduling_problem/1".to_string(),
        },
        algorithm: AlgorithmRef {
            id: algorithm.clone(),
            label: algorithm.to_uppercase(),
            version: PHD_VERSION.to_string(),
            config: algorithm_config,
        },
        run: RunInfo {
            run_id: "standalone".to_string(),
            kind: RunKind::MatrixCell,
            started_at: now.clone(),
            finished_at: now.clone(),
            status: RunStatus::Completed,
            exit_code: 0,
        },
        horizon: Horizon {
            start_mjd_utc: start_mjd,
            end_mjd_utc: end_mjd,
        },
        metrics,
        artifacts: scheduler::manifest::Artifacts {
            schedule: Some(ArtifactRef {
                uri: file_uri(schedule_path),
                size_bytes: schedule_size,
                sha256: schedule_sha,
                media_type: "application/json".to_string(),
            }),
            trace: None,
            problem: None,
        },
        links: scheduler::manifest::Links::default(),
        provenance: Provenance {
            matrix_run_id: None,
            cell_id: None,
            parent_manifest: None,
            repo_root: None,
            cli_args: std::env::args().collect(),
        },
        validation: ValidationReport {
            status: ValidationStatus::Valid,
            issues: Vec::new(),
        },
        extensions: serde_json::Value::Null,
    };

    let serialized = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("failed to serialize manifest: {e}"))?;

    if let Some(out_path) = out {
        fs::write(out_path, &serialized)
            .map_err(|e| format!("failed to write {}: {e}", out_path.display()))?;
        println!("phd manifest create: → {}", out_path.display());
    } else {
        println!("{serialized}");
    }
    Ok(ExitCode::SUCCESS)
}

fn build_cell_manifest(
    run_dir: &Path,
    experiment_name: &str,
    run_id: &str,
    now: &str,
    dataset_lookup: &std::collections::HashMap<&str, &DatasetSpecLite>,
    cell: &MatrixCellLite,
) -> Result<Manifest, String> {
    let metrics_path = run_dir
        .join("metrics")
        .join(format!("{}.json", cell.cell_id));
    let schedule_path = run_dir
        .join("schedules")
        .join(format!("{}.json", cell.cell_id));

    let metrics_text = fs::read_to_string(&metrics_path)
        .map_err(|e| format!("missing metrics ({}): {e}", metrics_path.display()))?;
    let metrics: ScheduleMetrics = serde_json::from_str(&metrics_text)
        .map_err(|e| format!("invalid metrics in {}: {e}", metrics_path.display()))?;

    let dataset_meta = dataset_lookup.get(cell.dataset_id.as_str());
    let dataset_path: PathBuf = dataset_meta
        .map(|d| d.path.clone())
        .unwrap_or_else(|| cell.dataset_path.clone());
    let dataset_sha = sha256_of(&dataset_path).unwrap_or_else(|_| "0".repeat(64));
    let dataset_name = dataset_meta
        .and_then(|d| d.label.clone())
        .unwrap_or_else(|| {
            cell.dataset_label
                .clone()
                .unwrap_or_else(|| cell.dataset_id.clone())
        });

    let schedule_artifact = if schedule_path.exists() {
        let bytes = fs::metadata(&schedule_path).map(|m| m.len()).unwrap_or(0);
        let sha = sha256_of(&schedule_path).unwrap_or_else(|_| "0".repeat(64));
        Some(ArtifactRef {
            uri: file_uri(&schedule_path),
            size_bytes: bytes,
            sha256: sha,
            media_type: "application/json".to_string(),
        })
    } else {
        None
    };

    let horizon = horizon_from_metrics(&metrics);

    let manifest = Manifest {
        manifest_schema_version: scheduler::manifest::MANIFEST_SCHEMA_VERSION.to_string(),
        manifest_id: uuid::Uuid::new_v4().to_string(),
        created_at: now.to_string(),
        producer: Producer {
            name: "phd".to_string(),
            version: PHD_VERSION.to_string(),
            git_sha: GIT_SHA.map(str::to_string),
            host: hostname(),
        },
        dataset: DatasetRef {
            id: cell.dataset_id.clone(),
            name: dataset_name,
            source_path: dataset_path.display().to_string(),
            sha256: dataset_sha,
            schema_version: "scheduling_problem/1".to_string(),
        },
        algorithm: AlgorithmRef {
            id: cell.algorithm.clone(),
            label: cell.algorithm.to_uppercase(),
            version: PHD_VERSION.to_string(),
            config: cell.run_config.clone(),
        },
        run: RunInfo {
            run_id: run_id.to_string(),
            kind: RunKind::MatrixCell,
            started_at: now.to_string(),
            finished_at: now.to_string(),
            status: RunStatus::Completed,
            exit_code: 0,
        },
        horizon,
        metrics,
        artifacts: scheduler::manifest::Artifacts {
            schedule: schedule_artifact,
            trace: None,
            problem: None,
        },
        links: scheduler::manifest::Links::default(),
        provenance: Provenance {
            matrix_run_id: Some(run_id.to_string()),
            cell_id: Some(cell.cell_id.clone()),
            parent_manifest: None,
            repo_root: None,
            cli_args: std::env::args().collect(),
        },
        validation: ValidationReport {
            status: ValidationStatus::Valid,
            issues: Vec::new(),
        },
        extensions: serde_json::Value::Null,
    };
    let _ = experiment_name; // currently unused but reserved for future provenance fields.
    Ok(manifest)
}

fn horizon_from_metrics(metrics: &ScheduleMetrics) -> Horizon {
    // The matrix runner does not currently surface horizon MJDs in
    // `metrics/<cell>.json`; we record total_horizon_sec under
    // `start=0, end=duration_in_days` as a stable, lossless placeholder
    // that downstream consumers can recognise (start==0).
    let days = metrics.total_horizon_sec / 86_400.0;
    Horizon {
        start_mjd_utc: 0.0,
        end_mjd_utc: days,
    }
}

// ---------------------------------------------------------------------------
// `phd manifest validate`
// ---------------------------------------------------------------------------

fn manifest_validate(path: &Path) -> Result<ExitCode, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|e| format!("invalid manifest JSON {}: {e}", path.display()))?;
    let report = manifest.validate();
    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("failed to serialize report: {e}"))?;
    println!("{json}");
    Ok(match report.status {
        ValidationStatus::Valid | ValidationStatus::Warning => ExitCode::SUCCESS,
        ValidationStatus::Invalid => ExitCode::FAILURE,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sha256_of(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

fn file_uri(path: &Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", abs.display())
}

fn hostname() -> Option<String> {
    std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty())
}

fn current_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------------------
// `phd publish`
// ---------------------------------------------------------------------------

fn publish(args: PublishArgs) -> Result<ExitCode, String> {
    if args.manifest.is_none() && args.manifest_dir.is_none() {
        return Err("either --manifest <FILE> or --manifest-dir <DIR> is required".into());
    }
    let base = args
        .url
        .clone()
        .or_else(|| std::env::var("PHD_WEBAPP_URL").ok())
        .unwrap_or_else(|| "http://localhost:8080".to_string());
    let base = base.trim_end_matches('/').to_string();
    let token = args
        .token
        .clone()
        .or_else(|| std::env::var("PHD_WEBAPP_TOKEN").ok());

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build();

    if args.create_workspace {
        let name = args
            .workspace_name
            .clone()
            .unwrap_or_else(|| args.workspace.clone());
        match create_workspace_call(&agent, &base, token.as_deref(), &args.workspace, &name) {
            Ok(()) => println!("workspace `{}` ready", args.workspace),
            Err(PublishError::Conflict) => {
                println!("workspace `{}` already exists, reusing", args.workspace);
            }
            Err(e) => return Err(e.to_string()),
        }
    }

    let manifests = collect_manifest_paths(&args)?;
    if manifests.is_empty() {
        return Err("no manifest files found".into());
    }

    let mut created = 0usize;
    let mut deduped = 0usize;
    let mut failed = 0usize;
    for path in &manifests {
        match publish_one(
            &agent,
            &base,
            token.as_deref(),
            &args.workspace,
            path,
            args.retries,
        ) {
            Ok(true) => {
                created += 1;
                println!("  + {}  (created)", path.display());
            }
            Ok(false) => {
                deduped += 1;
                println!("  = {}  (already present)", path.display());
            }
            Err(e) => {
                failed += 1;
                eprintln!("  ! {}: {e}", path.display());
            }
        }
    }
    println!(
        "phd publish: {} created, {} deduplicated, {} failed → {} (workspace `{}`)",
        created, deduped, failed, base, args.workspace
    );
    Ok(if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

#[derive(Debug)]
enum PublishError {
    Http(u16, String),
    Conflict,
    Transport(String),
    Body(String),
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(s, m) => write!(f, "HTTP {s}: {m}"),
            Self::Conflict => write!(f, "conflict"),
            Self::Transport(m) => write!(f, "transport: {m}"),
            Self::Body(m) => write!(f, "body: {m}"),
        }
    }
}

fn collect_manifest_paths(args: &PublishArgs) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if let Some(p) = &args.manifest {
        if !p.is_file() {
            return Err(format!("manifest file `{}` not found", p.display()));
        }
        out.push(p.clone());
    }
    if let Some(dir) = &args.manifest_dir {
        if !dir.is_dir() {
            return Err(format!("manifest dir `{}` not found", dir.display()));
        }
        walk_manifests(dir, &mut out);
        out.sort();
    }
    Ok(out)
}

fn walk_manifests(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_manifests(&p, out);
        } else if p.file_name().and_then(|s| s.to_str()) == Some("manifest.json") {
            out.push(p);
        }
    }
}

fn create_workspace_call(
    agent: &ureq::Agent,
    base: &str,
    token: Option<&str>,
    id: &str,
    name: &str,
) -> Result<(), PublishError> {
    let url = format!("{base}/v1/workspaces");
    let mut req = agent.post(&url);
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let body = serde_json::json!({ "name": name });
    match req.send_json(body) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(409, _)) => {
            // Slug derived server-side; verify the requested id matches by fetching the ws.
            let _ = id;
            Err(PublishError::Conflict)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(PublishError::Http(code, body))
        }
        Err(e) => Err(PublishError::Transport(e.to_string())),
    }
}

fn publish_one(
    agent: &ureq::Agent,
    base: &str,
    token: Option<&str>,
    workspace: &str,
    path: &Path,
    retries: u32,
) -> Result<bool, PublishError> {
    let bytes = fs::read(path).map_err(|e| PublishError::Body(e.to_string()))?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| PublishError::Body(e.to_string()))?;
    let key = manifest
        .get("manifest_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| sha256_short(&bytes));
    let payload = serde_json::json!({
        "manifest": manifest,
        "idempotency_key": key,
    });
    let url = format!("{base}/v1/workspaces/{workspace}/manifests");

    let mut attempt = 0u32;
    loop {
        let mut req = agent.post(&url);
        if let Some(t) = token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        match req.send_json(payload.clone()) {
            Ok(resp) => {
                let status = resp.status();
                let body: serde_json::Value = resp
                    .into_json()
                    .map_err(|e| PublishError::Body(e.to_string()))?;
                let created = body
                    .get("created")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let _ = status;
                return Ok(created);
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                if (500..600).contains(&code) && attempt < retries {
                    attempt += 1;
                    std::thread::sleep(backoff(attempt));
                    continue;
                }
                return Err(PublishError::Http(code, body));
            }
            Err(e) => {
                if attempt < retries {
                    attempt += 1;
                    std::thread::sleep(backoff(attempt));
                    continue;
                }
                return Err(PublishError::Transport(e.to_string()));
            }
        }
    }
}

fn backoff(attempt: u32) -> std::time::Duration {
    let secs = 1u64 << (attempt - 1).min(4);
    std::time::Duration::from_millis(secs * 250)
}

fn sha256_short(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let full = format!("{:x}", h.finalize());
    full[..16].to_string()
}
