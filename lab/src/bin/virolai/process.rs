//! Sibling binary discovery and subprocess dispatch.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

pub(crate) fn exec_sibling(name: &str, args: &[String]) -> Result<ExitCode, String> {
    let mut cmd = sibling_command(name);
    let status = cmd
        .args(args)
        .status()
        .map_err(|e| format!("failed to spawn `{name}`: {e}"))?;
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

pub(crate) fn sibling_command(name: &str) -> Command {
    if let Some(exe) = locate_sibling(name) {
        Command::new(exe)
    } else {
        cargo_run_command(name)
    }
}

fn locate_sibling(name: &str) -> Option<PathBuf> {
    if let Ok(current) = std::env::current_exe()
        && let Some(dir) = current.parent()
    {
        let candidate = dir.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        });
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn cargo_run_command(name: &str) -> Command {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(workspace_root());
    cmd.arg("run");
    if current_profile_is_release() {
        cmd.arg("--release");
    }
    match name {
        "schedulers" => {
            cmd.args(["-p", "schedulers", "--bin", "schedulers"]);
        }
        "lab" | "virolai" | "lab-ctao-adapter" => {
            cmd.args(["-p", "lab", "--bin", name]);
        }
        _ => {
            cmd.arg("--bin").arg(name);
        }
    }
    cmd.arg("--");
    cmd
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn current_profile_is_release() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .and_then(|dir| dir.file_name().map(|name| name == "release"))
        .unwrap_or(false)
}
