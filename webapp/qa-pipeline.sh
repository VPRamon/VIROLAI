#!/usr/bin/env bash
# Webapp QA pipeline
# ==================
# Runs lint, type-check, unit tests for both the TSI backend (Rust) and
# the TSI frontend (TypeScript / React). Designed to be cheap enough to
# run before every push, and to be invoked from the parent repo's
# `scripts/qa-pipeline.sh --with-webapp`.

set -euo pipefail

cd "$(dirname "$0")"

WEBAPP_ROOT=$(pwd)
WORKSPACE_ROOT="$(cd "${WEBAPP_ROOT}/.." && pwd)"
TSI_BACKEND="${WEBAPP_ROOT}/TSI/backend"
TSI_FRONTEND="${WEBAPP_ROOT}/TSI/frontend"

# Common feature set used by the rest of the toolchain. Tests run with
# the local in-memory repo so postgres is not required for QA.
TSI_FEATURES="local-repo,http-server"

log() { printf '\n=== %s ===\n' "$*"; }

log "Workspace webapp: cargo fmt --check"
(cd "${WORKSPACE_ROOT}" && cargo fmt --package webapp -- --check)

log "Workspace webapp: cargo clippy"
(cd "${WORKSPACE_ROOT}" && cargo clippy -p webapp --all-targets -- -D warnings)

log "Workspace webapp: cargo test"
(cd "${WORKSPACE_ROOT}" && cargo test -p webapp --all-features)

log "Backend: cargo fmt --check"
(cd "${TSI_BACKEND}" && cargo fmt --all -- --check)

log "Backend: cargo clippy (lib only, deny warnings)"
(cd "${TSI_BACKEND}" && cargo clippy --lib --no-default-features --features "${TSI_FEATURES}" -- -D warnings)

log "Backend: cargo test --lib"
(cd "${TSI_BACKEND}" && cargo test --lib --no-default-features --features "${TSI_FEATURES}")

log "Frontend: npm ci (only when node_modules missing)"
if [[ ! -d "${TSI_FRONTEND}/node_modules" ]]; then
    (cd "${TSI_FRONTEND}" && npm ci)
fi

log "Frontend: npm run lint"
(cd "${TSI_FRONTEND}" && npm run lint)

log "Frontend: npm run type-check"
(cd "${TSI_FRONTEND}" && npm run type-check)

log "Frontend: npm run test:run"
(cd "${TSI_FRONTEND}" && npm run test:run)

log "Webapp QA pipeline complete ✓"
