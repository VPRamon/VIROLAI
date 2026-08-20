#!/usr/bin/env bash
# Compatibility wrapper around `virolai publish`.
#
# Usage:
#   upload_results.sh --workspace <id> --dir <DIR> [options]
#
# Supported options are forwarded to `virolai publish`. Run
# `virolai publish --help` for the authoritative list.

set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    cat <<'EOF'
Usage:
  upload_results.sh --workspace <id> --dir <DIR> [options]

Common options:
  --workspace <id>          Target workspace.
  --dir <DIR>               Directory containing schedule JSON files.
  --url <URL>               Webapp base URL, or VIROLAI_WEBAPP_URL.
  --token <TOKEN>           Bearer token, or VIROLAI_WEBAPP_TOKEN.
  --create-workspace        Create the workspace if missing.
  --workspace-name <NAME>   Display name when creating the workspace.
  --retries <N>             Maximum retry attempts.
EOF
    exit 0
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
VIROLAI_BIN=""

for candidate in \
    "${ROOT_DIR}/target/release/virolai" \
    "${ROOT_DIR}/target/debug/virolai"; do
    if [[ -x "${candidate}" ]]; then
        VIROLAI_BIN="${candidate}"
        break
    fi
done

if [[ -z "${VIROLAI_BIN}" ]]; then
    if command -v virolai >/dev/null 2>&1; then
        VIROLAI_BIN="$(command -v virolai)"
    else
        echo "upload_results.sh: cannot find the \`virolai\` binary." >&2
        echo "Build it with \`cargo build -p lab --bin virolai\` or add it to PATH." >&2
        exit 127
    fi
fi

exec "${VIROLAI_BIN}" publish "$@"
