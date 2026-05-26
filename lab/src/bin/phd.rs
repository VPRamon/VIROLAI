//! `phd` — unified user-facing CLI for the PhD scheduling workspace.
//!
//! - `phd run`    — delegates to the `schedulers` binary.
//! - `phd matrix` — delegates to the `lab` binary.
//! - `phd sweep`  — wrapper around `lab run` that executes an experiment spec
//!   and stores all results in SQLite. Supports `--parallel` and `--override`.
//! - `phd dataset adapt` — delegates to `lab-ctao-adapter`.
//! - `phd publish` — uploads schedule JSONs from a directory to the webapp
//!   `/v1/workspaces/{id}/schedules/batch` endpoint with idempotency,
//!   chunked batches and exponential-backoff retries.
//!
//! Subprocess dispatch uses the sibling binary that lives next to `phd` in the
//! same directory (the cargo `target/.../` layout). When that lookup fails the
//! CLI falls back to the binary name on `$PATH`.

use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const PHD_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(
    name = "phd",
    version,
    about = "PhD scheduling CLI — run simulations, publish to the webapp",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run a single scheduling problem (delegates to the `schedulers` binary).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Run {
        /// Forwarded as-is to the `schedulers` binary.
        args: Vec<String>,
    },
    /// Run a sweep / matrix experiment (delegates to `lab`).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Matrix {
        /// Forwarded as-is to the `lab` binary.
        args: Vec<String>,
    },
    /// Run a sweep and store results in SQLite — the canonical way to run experiments.
    ///
    /// Executes the experiment defined in SPEC via `lab run`, storing schedule
    /// JSON and metrics for each cell in the registry database. Cells already
    /// present in the DB are skipped unless `--override` is set.
    Sweep {
        /// Path to the experiment spec JSON (same format as `lab run --spec`).
        #[arg(long, value_name = "FILE")]
        spec: PathBuf,
        /// Path to the registry SQLite database (default: `.lab/runs.sqlite`).
        #[arg(long, value_name = "PATH")]
        run_db: Option<PathBuf>,
        /// Override parallelism (threads). Defaults to spec's `max_parallel`.
        #[arg(long, value_name = "N")]
        parallel: Option<usize>,
        /// Re-execute cells that are already present in the DB and update their row.
        #[arg(long = "override")]
        override_existing: bool,
    },
    /// Dataset utilities.
    Dataset {
        #[command(subcommand)]
        cmd: DatasetCmd,
    },
    /// Upload schedule JSONs from a directory to a webapp workspace.
    Publish(PublishArgs),
}

#[derive(Subcommand, Debug)]
enum DatasetCmd {
    /// CTA-O dataset adapter (delegates to `lab-ctao-adapter`).
    #[command(
        disable_help_flag = true,
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    Adapt { args: Vec<String> },
}

#[derive(Parser, Debug)]
struct PublishArgs {
    /// Target workspace id (slug). Created if `--create-workspace` is set.
    #[arg(long)]
    workspace: String,
    /// Directory containing schedule JSON files (searched recursively).
    #[arg(long, value_name = "DIR")]
    dir: PathBuf,
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
        Cmd::Run { args } => exec_sibling("schedulers", &args),
        Cmd::Matrix { args } => exec_sibling("lab", &args),
        Cmd::Sweep {
            spec,
            run_db,
            parallel,
            override_existing,
        } => sweep(&spec, run_db.as_deref(), parallel, override_existing),
        Cmd::Dataset {
            cmd: DatasetCmd::Adapt { args },
        } => exec_sibling("lab-ctao-adapter", &args),
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
    run_db: Option<&Path>,
    parallel_override: Option<usize>,
    override_existing: bool,
) -> Result<ExitCode, String> {
    if !spec_path.is_file() {
        return Err(format!("spec file `{}` not found", spec_path.display()));
    }

    // If --parallel was requested, write a patched spec to a temp dir so that
    // `lab run` picks up the overridden max_parallel without modifying the
    // original file.
    let (effective_spec, _tmp): (PathBuf, Option<tempfile::TempDir>) =
        if let Some(n) = parallel_override {
            let tmp =
                tempfile::TempDir::new().map_err(|e| format!("failed to create temp dir: {e}"))?;
            let patched = patch_spec_parallel(spec_path, tmp.path(), n)?;
            (patched, Some(tmp))
        } else {
            (spec_path.to_path_buf(), None)
        };

    let mut cmd = Command::new(locate_sibling("lab"));
    cmd.arg("run").arg("--spec").arg(&effective_spec);
    if let Some(db) = run_db {
        cmd.arg("--run-db").arg(db);
    }
    if override_existing {
        cmd.arg("--override");
    }

    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn lab: {e}"))?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Write a copy of `spec_path` to `<tmp_dir>/spec_patched.json` with
/// `max_parallel` overridden to `n`. Dataset paths are resolved to absolute
/// so the patched spec remains valid when run from a different working directory.
fn patch_spec_parallel(spec_path: &Path, tmp_dir: &Path, n: usize) -> Result<PathBuf, String> {
    let text = fs::read_to_string(spec_path)
        .map_err(|e| format!("failed to read spec `{}`: {e}", spec_path.display()))?;
    let mut v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("failed to parse spec `{}`: {e}", spec_path.display()))?;

    if let Some(obj) = v.as_object_mut() {
        obj.insert("max_parallel".to_string(), serde_json::json!(n));
        let base = spec_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        resolve_dataset_paths(obj, &base);
    }

    let patched_path = tmp_dir.join("spec_patched.json");
    let patched = serde_json::to_string_pretty(&v)
        .map_err(|e| format!("failed to serialize patched spec: {e}"))?;
    fs::write(&patched_path, patched).map_err(|e| format!("failed to write patched spec: {e}"))?;
    Ok(patched_path)
}

fn resolve_dataset_paths(obj: &mut serde_json::Map<String, serde_json::Value>, base: &Path) {
    // datasets array: each item may have a "path" field.
    if let Some(serde_json::Value::Array(datasets)) = obj.get_mut("datasets") {
        for dataset in datasets.iter_mut() {
            if let Some(p) = dataset.get("path").and_then(|v| v.as_str()) {
                let abs = resolve_relative(base, Path::new(p));
                if let Some(obj2) = dataset.as_object_mut() {
                    obj2.insert(
                        "path".to_string(),
                        serde_json::Value::String(abs.display().to_string()),
                    );
                }
            }
        }
    }
    // matrix.datasets array (alternate schema shape).
    if let Some(serde_json::Value::Object(matrix)) = obj.get_mut("matrix")
        && let Some(serde_json::Value::Array(datasets)) = matrix.get_mut("datasets")
    {
        for dataset in datasets.iter_mut() {
            if let Some(p) = dataset.get("path").and_then(|v| v.as_str()) {
                let abs = resolve_relative(base, Path::new(p));
                if let Some(obj2) = dataset.as_object_mut() {
                    obj2.insert(
                        "path".to_string(),
                        serde_json::Value::String(abs.display().to_string()),
                    );
                }
            }
        }
    }
}

fn resolve_relative(base: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        let joined = base.join(p);
        fs::canonicalize(&joined).unwrap_or(joined)
    }
}

// ---------------------------------------------------------------------------
// `phd publish`
// ---------------------------------------------------------------------------

fn publish(args: PublishArgs) -> Result<ExitCode, String> {
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
        .timeout(std::time::Duration::from_secs(60))
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

    let schedule_paths = collect_schedule_paths(&args.dir)?;
    if schedule_paths.is_empty() {
        return Err(format!(
            "no schedule JSON files found under `{}`",
            args.dir.display()
        ));
    }
    println!(
        "phd publish: {} schedule(s) found under `{}`",
        schedule_paths.len(),
        args.dir.display()
    );

    const SCHEDULE_CHUNK_SIZE: usize = 25;
    let mut total_created = 0usize;
    let mut total_deduped = 0usize;
    let mut total_failed = 0usize;
    let total = schedule_paths.len();
    let mut done = 0usize;

    for chunk in schedule_paths.chunks(SCHEDULE_CHUNK_SIZE) {
        match publish_schedules_batch_chunk(
            &agent,
            &base,
            token.as_deref(),
            &args.workspace,
            chunk,
            args.retries,
        ) {
            Ok((c, d, f)) => {
                total_created += c;
                total_deduped += d;
                total_failed += f;
            }
            Err(e) => {
                eprintln!("  ! schedule batch failed: {e}");
                total_failed += chunk.len();
            }
        }
        done += chunk.len();
        eprintln!("[{done}/{total}] schedules uploaded");
    }

    println!(
        "phd publish: {} created, {} deduplicated, {} failed → {} (workspace `{}`)",
        total_created, total_deduped, total_failed, base, args.workspace
    );
    Ok(if total_failed > 0 {
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

/// Walk DIR and collect `.json` files that look like self-contained schedules.
fn collect_schedule_paths(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Err(format!("dir `{}` not found", dir.display()));
    }
    let mut all = Vec::new();
    walk_json(dir, &mut all);
    all.sort();

    let mut schedules = Vec::new();
    for path in all {
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  · skipping `{}`: {e}", path.display());
                continue;
            }
        };
        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("  · skipping `{}` (invalid JSON): {e}", path.display());
                continue;
            }
        };
        // A self-contained schedule has schedule_metadata and a blocks list.
        if value.get("schedule_metadata").is_some()
            && (value.get("scheduling_blocks").is_some() || value.get("blocks").is_some())
        {
            schedules.push(path);
        } else {
            eprintln!(
                "  · skipping `{}` (not a self-contained schedule)",
                path.display()
            );
        }
    }
    Ok(schedules)
}

fn walk_json(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_json(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("json") {
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

/// Upload a batch of self-contained schedules to the
/// `/v1/workspaces/{workspace}/schedules/batch` endpoint. Returns `(created, deduped, failed)`.
fn publish_schedules_batch_chunk(
    agent: &ureq::Agent,
    base: &str,
    token: Option<&str>,
    workspace: &str,
    paths: &[PathBuf],
    retries: u32,
) -> Result<(usize, usize, usize), PublishError> {
    let mut items: Vec<serde_json::Value> = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(path).map_err(|e| PublishError::Body(e.to_string()))?;
        let schedule: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| PublishError::Body(e.to_string()))?;
        items.push(serde_json::json!({
            "schedule": schedule,
            "idempotency_key": sha256_short(&bytes),
        }));
    }
    let payload = serde_json::json!({ "items": items });
    let url = format!("{base}/v1/workspaces/{workspace}/schedules/batch");

    let mut attempt = 0u32;
    loop {
        let mut req = agent.post(&url);
        if let Some(t) = token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        match req.send_json(payload.clone()) {
            Ok(resp) => {
                let body: serde_json::Value = resp
                    .into_json()
                    .map_err(|e| PublishError::Body(e.to_string()))?;
                let summary = body.get("summary").cloned().unwrap_or_default();
                let created = summary.get("created").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let deduped = summary
                    .get("deduplicated")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let failed = summary.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                return Ok((created, deduped, failed));
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
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let full = format!("{:x}", h.finalize());
    full[..16].to_string()
}

// ---------------------------------------------------------------------------
// Version constant — exposed for tests
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn phd_version() -> &'static str {
    PHD_VERSION
}
