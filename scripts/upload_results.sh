#!/usr/bin/env bash
# scripts/upload_results.sh — thin wrapper around `phd publish`.
#
# History note: previous versions of this script issued raw HTTP
# requests against the workspaces backend. That logic has been
# consolidated into the `phd publish` subcommand (idempotency, retries,
# chunked batches, and mixed manifest/schedule classification all live
# there now). This wrapper exists for back-compat with shell pipelines
# that already invoke it; new callers should prefer `phd publish`
# directly.
#
# Usage:
#     upload_results.sh --workspace <id> --dir <DIR> [options]
#     upload_results.sh --workspace <id> --manifest <FILE> [options]
#
# Options forwarded verbatim to `phd publish`:
#     --workspace <id>          Target workspace (required).
#     --dir <DIR>               Recurse DIR; classify each .json as
#                               manifest or self-contained schedule.
#     --manifest <FILE>         Publish a single manifest file.
#     --url <URL>               Webapp base URL (or $PHD_WEBAPP_URL).
#     --token <TOKEN>           Bearer token (or $PHD_WEBAPP_TOKEN).
#     --create-workspace        Create the workspace if missing.
#     --workspace-name <NAME>   Display name when creating.
#     --include-schedules <bool>  Persist full schedules (default: true).
#     --retries <N>             Max retry attempts (default: 3).
#
# Examples:
#     # Publish an entire sweep output directory (manifests + schedules).
#     scripts/upload_results.sh --workspace paper --dir out/my-sweep --create-workspace
#
#     # Manifests only (smaller payloads, no drill-down later).
#     scripts/upload_results.sh --workspace paper --dir out/my-sweep --include-schedules false

set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    sed -n '2,30p' "$0"
    exit 0
fi

# Locate the `phd` binary: prefer cargo workspace builds when present,
# otherwise rely on PATH.
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
PHD_BIN=""
for candidate in \
    "${ROOT_DIR}/target/release/phd" \
    "${ROOT_DIR}/target/debug/phd"; do
    if [[ -x "${candidate}" ]]; then
        PHD_BIN="${candidate}"
        break
    fi
done

if [[ -z "${PHD_BIN}" ]]; then
    if command -v phd >/dev/null 2>&1; then
        PHD_BIN="$(command -v phd)"
    else
        echo "upload_results.sh: cannot find the \`phd\` binary." >&2
        echo "Build it with \`cargo build -p lab --bin phd\` or add it to PATH." >&2
        exit 127
    fi
fi

exec "${PHD_BIN}" publish "$@"
