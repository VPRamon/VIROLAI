//! Child-process orchestrator for the `experiment_matrix` binary.
//!
//! `submit` spawns a tokio child process; we keep a per-run handle so we
//! can cancel (SIGTERM on Unix) and report aggregate status. The matrix
//! creates its own `run-<ts>` directory; we discover it by snapshotting
//! the slug directory before spawning and polling for the new entry.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::fs as tfs;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::experiments::catalog::{Catalog, RunStatus};
use crate::experiments::errors::{ExperimentError, ExperimentResult};

/// Returned by [`ExperimentRunner::submit`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubmitResult {
    pub experiment_slug: String,
    pub run_id: String,
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunHandleStatus {
    Running,
    Completed,
    Failed,
    NotFound,
}

struct RunHandle {
    child: Mutex<Option<Child>>,
    pid: Option<u32>,
    /// Last known exit status: None=still running, Some(true)=success.
    exit_success: RwLock<Option<bool>>,
}

pub struct ExperimentRunner {
    root: PathBuf,
    bin_override: Option<String>,
    semaphore: Arc<Semaphore>,
    handles: RwLock<HashMap<(String, String), Arc<RunHandle>>>,
    catalog: Arc<Catalog>,
}

impl ExperimentRunner {
    pub fn new(
        root: PathBuf,
        bin_override: Option<String>,
        max_concurrent: usize,
        catalog: Arc<Catalog>,
    ) -> Self {
        Self {
            root,
            bin_override,
            semaphore: Arc::new(Semaphore::new(max_concurrent.max(1))),
            handles: RwLock::new(HashMap::new()),
            catalog,
        }
    }

    /// Submit a fresh experiment. Writes the spec to a staging file under
    /// `<root>/.staging/`, spawns the matrix, then waits up to 10s for the
    /// matrix to create its `run-*` directory under `<root>/<slug>/` so we
    /// can return a stable run_id.
    pub async fn submit(&self, spec_json: Value) -> ExperimentResult<SubmitResult> {
        // 1) Validate basic spec shape.
        let name = spec_json
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ExperimentError::Unprocessable("spec missing `name`".into()))?
            .to_string();
        if spec_json
            .get("datasets")
            .and_then(|v| v.as_array())
            .is_none_or(|a| a.is_empty())
        {
            return Err(ExperimentError::Unprocessable(
                "spec must declare at least one dataset".into(),
            ));
        }
        if spec_json
            .get("algorithms")
            .and_then(|v| v.as_array())
            .is_none_or(|a| a.is_empty())
        {
            return Err(ExperimentError::Unprocessable(
                "spec must declare at least one algorithm".into(),
            ));
        }

        let slug = experiment_slug(&name);
        let exp_dir = self.root.join(&slug);
        tfs::create_dir_all(&exp_dir).await?;

        // 2) Force `output_dir` to our root so the matrix lands under
        //    `<root>/<slug>/run-*/`. We rewrite the spec on disk; the
        //    user's original spec is preserved alongside it for audit.
        let mut spec_for_matrix = spec_json.clone();
        spec_for_matrix
            .as_object_mut()
            .ok_or_else(|| ExperimentError::Unprocessable("spec must be a JSON object".into()))?
            .insert(
                "output_dir".to_string(),
                Value::String(self.root.to_string_lossy().to_string()),
            );

        let staging = self.root.join(".staging");
        tfs::create_dir_all(&staging).await?;
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%9f").to_string();
        let staging_spec = staging.join(format!("{slug}-{stamp}.json"));
        tfs::write(&staging_spec, serde_json::to_vec_pretty(&spec_for_matrix)?).await?;

        // 3) Snapshot existing run-* dirs so we can detect the new one.
        let existing = list_run_dirs(&exp_dir).await?;

        // 4) Spawn the matrix under a concurrency permit.
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| ExperimentError::Internal(format!("semaphore closed: {e}")))?;

        let bin = self.matrix_command();
        let (program, leading_args) = bin;

        let mut cmd = Command::new(&program);
        cmd.args(&leading_args);
        cmd.arg("--spec").arg(&staging_spec);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let child = cmd.spawn().map_err(|e| {
            ExperimentError::Internal(format!(
                "failed to spawn experiment_matrix `{}`: {e}",
                program
            ))
        })?;

        // 5) Discover the freshly-created run dir (poll up to 10s).
        let run_id = match wait_for_new_run(&exp_dir, &existing, Duration::from_secs(10)).await {
            Some(id) => id,
            None => {
                // The matrix never produced a run dir within the timeout;
                // it almost certainly crashed during startup. Reap it now.
                drop(permit);
                let mut child = child;
                let _ = child.kill().await;
                let output = child.wait_with_output().await.ok();
                let stderr = output
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stderr).to_string())
                    .unwrap_or_default();
                return Err(ExperimentError::Internal(format!(
                    "experiment_matrix did not create a run directory under {} within 10s; stderr: {}",
                    exp_dir.display(),
                    stderr
                )));
            }
        };

        let run_dir = exp_dir.join(&run_id);

        // 6) Persist the user's original spec for audit.
        let _ = tfs::write(
            run_dir.join("spec.json"),
            serde_json::to_vec_pretty(&spec_json)?,
        )
        .await;

        // 7) Track the child and supervise it.
        let pid = child.id();
        let handle = Arc::new(RunHandle {
            child: Mutex::new(Some(child)),
            pid,
            exit_success: RwLock::new(None),
        });
        {
            let mut guard = self.handles.write().await;
            guard.insert((slug.clone(), run_id.clone()), handle.clone());
        }

        // Forward stdout/stderr to log files in the run dir.
        if let Ok(mut child_guard) = handle.child.try_lock()
            && let Some(child) = child_guard.as_mut()
        {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            spawn_log_pump(stdout, run_dir.join("stdout.log"));
            spawn_log_pump(stderr, run_dir.join("stderr.log"));
        }

        // Supervisor task: wait for exit, record success, refresh catalog.
        let supervisor_handle = handle.clone();
        let supervisor_catalog = self.catalog.clone();
        let supervisor_slug = slug.clone();
        let supervisor_run_id = run_id.clone();
        tokio::spawn(async move {
            let mut taken = supervisor_handle.child.lock().await.take();
            let exit = if let Some(child) = taken.as_mut() {
                child.wait().await.ok().map(|s| s.success())
            } else {
                None
            };
            *supervisor_handle.exit_success.write().await = exit;
            // Force a catalog refresh so the new run is visible right away.
            let _ = supervisor_catalog.refresh();
            drop(permit);
            tracing::info!(
                "experiment_matrix child for {}/{} exited (success={:?})",
                supervisor_slug,
                supervisor_run_id,
                exit
            );
        });

        // Force one immediate refresh so list_experiments sees the run.
        let _ = self.catalog.refresh();

        Ok(SubmitResult {
            experiment_slug: slug,
            run_id,
            output_dir: run_dir,
        })
    }

    /// Resume an existing run (re-spawn `experiment_matrix --resume <dir>`).
    pub async fn resume(&self, slug: &str, run_id: &str) -> ExperimentResult<SubmitResult> {
        let run_dir = self.root.join(slug).join(run_id);
        if !run_dir.exists() {
            return Err(ExperimentError::NotFound(format!(
                "run {slug}/{run_id} not found"
            )));
        }
        // Spec for resume must come from spec.json (or the manifest's
        // `spec` field as a fallback).
        let spec_path = run_dir.join("spec.json");
        let manifest_path = run_dir.join("experiment.json");
        let spec_arg = if spec_path.exists() {
            spec_path.clone()
        } else if manifest_path.exists() {
            // Extract `spec` from manifest into a sibling spec.json.
            let text = tfs::read_to_string(&manifest_path).await?;
            let value: Value = serde_json::from_str(&text)?;
            let spec = value
                .get("spec")
                .cloned()
                .ok_or_else(|| ExperimentError::Internal("manifest missing `spec`".into()))?;
            tfs::write(&spec_path, serde_json::to_vec_pretty(&spec)?).await?;
            spec_path.clone()
        } else {
            return Err(ExperimentError::BadRequest(
                "no spec.json or experiment.json found for resume".into(),
            ));
        };

        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| ExperimentError::Internal(format!("semaphore closed: {e}")))?;

        let (program, leading_args) = self.matrix_command();
        let mut cmd = Command::new(&program);
        cmd.args(&leading_args);
        cmd.arg("--spec").arg(&spec_arg);
        cmd.arg("--resume").arg(&run_dir);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| {
            ExperimentError::Internal(format!("failed to spawn experiment_matrix: {e}"))
        })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        spawn_log_pump(stdout, run_dir.join("stdout-resume.log"));
        spawn_log_pump(stderr, run_dir.join("stderr-resume.log"));

        let pid = child.id();
        let handle = Arc::new(RunHandle {
            child: Mutex::new(Some(child)),
            pid,
            exit_success: RwLock::new(None),
        });
        {
            let mut guard = self.handles.write().await;
            guard.insert((slug.to_string(), run_id.to_string()), handle.clone());
        }
        let supervisor_handle = handle.clone();
        let supervisor_catalog = self.catalog.clone();
        tokio::spawn(async move {
            let mut taken = supervisor_handle.child.lock().await.take();
            let exit = if let Some(child) = taken.as_mut() {
                child.wait().await.ok().map(|s| s.success())
            } else {
                None
            };
            *supervisor_handle.exit_success.write().await = exit;
            let _ = supervisor_catalog.refresh();
            drop(permit);
        });

        Ok(SubmitResult {
            experiment_slug: slug.to_string(),
            run_id: run_id.to_string(),
            output_dir: run_dir,
        })
    }

    /// Send SIGTERM to the child (Unix) / kill (other platforms).
    pub async fn cancel(&self, slug: &str, run_id: &str) -> ExperimentResult<()> {
        let handle = {
            let guard = self.handles.read().await;
            guard.get(&(slug.to_string(), run_id.to_string())).cloned()
        };
        let Some(handle) = handle else {
            return Err(ExperimentError::Conflict(format!(
                "no live orchestration tracked for {slug}/{run_id}"
            )));
        };
        let exit = handle.exit_success.read().await;
        if exit.is_some() {
            return Err(ExperimentError::Conflict(format!(
                "{slug}/{run_id} has already exited"
            )));
        }
        drop(exit);

        #[cfg(unix)]
        if let Some(pid) = handle.pid {
            // SAFETY: libc::kill is a thin wrapper over the syscall; we
            // pass a known-valid PID. SIGTERM is non-fatal if the process
            // ignores it, so subsequent reads of the child status remain
            // safe.
            let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                return Err(ExperimentError::Internal(format!("kill failed: {err}")));
            }
            return Ok(());
        }

        // Non-Unix fallback: SIGKILL via tokio.
        #[cfg(not(unix))]
        {
            let mut guard = handle.child.lock().await;
            if let Some(child) = guard.as_mut() {
                let _ = child.start_kill();
            }
        }

        Ok(())
    }

    pub async fn status(&self, slug: &str, run_id: &str) -> RunHandleStatus {
        let handle = {
            let guard = self.handles.read().await;
            guard.get(&(slug.to_string(), run_id.to_string())).cloned()
        };
        if let Some(h) = handle {
            match *h.exit_success.read().await {
                Some(true) => RunHandleStatus::Completed,
                Some(false) => RunHandleStatus::Failed,
                None => RunHandleStatus::Running,
            }
        } else {
            // No live handle: derive from on-disk state.
            match self.catalog.get_experiment_index(slug, run_id) {
                Ok(idx) => match idx.status {
                    RunStatus::Running | RunStatus::Pending => RunHandleStatus::Running,
                    RunStatus::Completed => RunHandleStatus::Completed,
                    RunStatus::Failed => RunHandleStatus::Failed,
                },
                Err(_) => RunHandleStatus::NotFound,
            }
        }
    }

    /// Returns the (program, prepended_args) pair to invoke. Resolution
    /// order:
    ///   1. `PHD_EXPERIMENT_MATRIX_BIN` (split on whitespace).
    ///   2. Sibling of the current executable named `experiment_matrix`.
    ///   3. `cargo run --bin experiment_matrix --`.
    fn matrix_command(&self) -> (String, Vec<String>) {
        if let Some(bin) = self.bin_override.as_deref() {
            let mut parts = bin.split_whitespace();
            let prog = parts.next().unwrap_or("experiment_matrix").to_string();
            let rest: Vec<String> = parts.map(String::from).collect();
            return (prog, rest);
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(parent) = exe.parent()
        {
            let candidate = parent.join("experiment_matrix");
            if candidate.exists() {
                return (candidate.to_string_lossy().to_string(), Vec::new());
            }
        }
        (
            "cargo".to_string(),
            vec![
                "run".into(),
                "--quiet".into(),
                "--bin".into(),
                "experiment_matrix".into(),
                "--".into(),
            ],
        )
    }
}

fn spawn_log_pump<R>(reader: Option<R>, path: PathBuf)
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
{
    if let Some(mut r) = reader {
        tokio::spawn(async move {
            let file = match tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
            {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("failed to open log {}: {e}", path.display());
                    return;
                }
            };
            let mut writer = tokio::io::BufWriter::new(file);
            if let Err(e) = tokio::io::copy(&mut r, &mut writer).await {
                tracing::warn!("log pump for {} aborted: {e}", path.display());
            }
            use tokio::io::AsyncWriteExt;
            let _ = writer.flush().await;
        });
    }
}

async fn list_run_dirs(slug_dir: &Path) -> ExperimentResult<HashSet<String>> {
    let mut out = HashSet::new();
    if !slug_dir.exists() {
        return Ok(out);
    }
    let mut rd = tfs::read_dir(slug_dir).await?;
    while let Some(entry) = rd.next_entry().await? {
        let ft = entry.file_type().await?;
        if ft.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("run-") {
                out.insert(name);
            }
        }
    }
    Ok(out)
}

async fn wait_for_new_run(
    slug_dir: &Path,
    existing: &HashSet<String>,
    timeout: Duration,
) -> Option<String> {
    let deadline = Instant::now() + timeout;
    let mut interval = tokio::time::interval(Duration::from_millis(50));
    while Instant::now() < deadline {
        interval.tick().await;
        let current = list_run_dirs(slug_dir).await.unwrap_or_default();
        let mut diff: Vec<_> = current.difference(existing).cloned().collect();
        if !diff.is_empty() {
            diff.sort();
            return diff.into_iter().next();
        }
    }
    None
}

/// Mirror of `scripts/experiment_matrix/output.rs::experiment_slug`.
pub fn experiment_slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "experiment".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_matches_matrix_definition() {
        assert_eq!(experiment_slug("hello world"), "hello-world");
        assert_eq!(experiment_slug("ctao/paper:matrix"), "ctao-paper-matrix");
        assert_eq!(experiment_slug("***"), "experiment");
    }
}
