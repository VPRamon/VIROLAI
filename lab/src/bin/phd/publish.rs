//! `phd publish` webapp upload workflow.

use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser, Debug)]
pub(crate) struct PublishArgs {
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

pub(crate) fn publish(args: PublishArgs) -> Result<ExitCode, String> {
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
        "phd publish: {} created, {} deduplicated, {} failed -> {} (workspace `{}`)",
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
                eprintln!("  . skipping `{}`: {e}", path.display());
                continue;
            }
        };
        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("  . skipping `{}` (invalid JSON): {e}", path.display());
                continue;
            }
        };
        if value.get("schedule_metadata").is_some()
            && (value.get("scheduling_blocks").is_some() || value.get("blocks").is_some())
        {
            schedules.push(path);
        } else {
            eprintln!(
                "  . skipping `{}` (not a self-contained schedule)",
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
