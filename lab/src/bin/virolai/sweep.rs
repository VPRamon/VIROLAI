//! `virolai sweep` wrapper around `lab run`.

use super::process::sibling_command;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub(crate) fn sweep(
    spec_path: &Path,
    run_db: Option<&Path>,
    parallel_override: Option<usize>,
    override_existing: bool,
) -> Result<ExitCode, String> {
    if !spec_path.is_file() {
        return Err(format!("spec file `{}` not found", spec_path.display()));
    }

    let (effective_spec, _tmp): (PathBuf, Option<tempfile::TempDir>) =
        if let Some(n) = parallel_override {
            let tmp =
                tempfile::TempDir::new().map_err(|e| format!("failed to create temp dir: {e}"))?;
            let patched = patch_spec_parallel(spec_path, tmp.path(), n)?;
            (patched, Some(tmp))
        } else {
            (spec_path.to_path_buf(), None)
        };

    let mut cmd = sibling_command("lab");
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
/// paths so the patched spec remains valid from the temporary directory.
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
