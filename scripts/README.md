# `scripts/`

Operational helpers around the scheduler crate and the webapp. Each
script is self-contained; together they cover the day-to-day research
loop.

| Script | Purpose |
|---|---|
| `qa-pipeline.sh` | Run `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test --all-features`. |
| `ctao_adapter.rs` | Convert raw CTA dataset files into a `scheduling_problem.json`. |
| `upload_results.sh` | Thin wrapper over `phd publish` (kept for shell-pipeline back-compat). |
| `phd_tsi_server` | Local backend for the webapp (also lives at `webapp/scripts/`). Run with `cargo run --bin phd_tsi_server`. |

---

## `qa-pipeline.sh`

Canonical pre-commit / pre-PR check. Equivalent to the three commands
in the project's `AGENTS.md`:

```bash
./scripts/qa-pipeline.sh
```

Fails fast on the first unsuccessful step.

---

## `ctao_adapter.rs`

Compiled binary that ingests `*_internalSDC.json` files produced by the
CTA dataset pipeline and emits a `scheduling_problem.json` validated
against `schemas/scheduling_problem/scheduling_problem.schema.json`.

```bash
cargo run --bin ctao_adapter -- \
    --input  data/raw/cta_n_internalSDC.json \
    --output data/cta_n.json
```

Notes:

- The output is deterministic for a fixed input.
- Re-run whenever the upstream dataset format bumps its version.

---

## `upload_results.sh`

Back-compat wrapper around `phd publish`. New scripts and humans should
prefer `cargo run --bin phd -- publish` directly. This wrapper exists
so legacy CI pipelines and shell aliases keep working.

```bash
scripts/upload_results.sh \
    --workspace paper-2024 \
    --dir       out/paper-sweep \
    --create-workspace \
    --include-schedules
```

All flags are forwarded to `phd publish`. See `phd publish --help` for
the authoritative list.

The previous version of this script issued raw HTTP requests against
endpoints that no longer exist (`POST /v1/schedules` and a manifest
payload missing the `idempotency_key` envelope). Those bugs are gone
because all HTTP work now lives in the Rust client.

---

## `phd_tsi_server`

The webapp backend. Runs the workspaces API plus the TSI adapter
endpoints. For local development:

```bash
PHD_WORKSPACES_DIR=./workspaces cargo run --bin phd_tsi_server
```

For production / Docker, see `webapp/setup.sh` and `webapp/docker/`.
