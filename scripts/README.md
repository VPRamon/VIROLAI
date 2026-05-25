# `scripts/`

Operational helpers around the schedulers crate and the webapp. Each
script is self-contained; together they cover the day-to-day research
loop.

| Script | Purpose |
|---|---|
| `qa-pipeline.sh` | Run workspace `cargo fmt --check`, `cargo clippy --workspace --exclude tsi-rust --all-targets -D warnings`, `cargo test --workspace --exclude tsi-rust --all-features`. |
| `lab-ctao-adapter` | Convert raw CTA dataset files into a `scheduling_problem.json`. |
| `upload_results.sh` | Thin wrapper over `phd publish` (kept for shell-pipeline back-compat). |
| `webapp` | Local backend for the webapp. Run with `cargo run -p webapp --bin webapp`. |

---

## `qa-pipeline.sh`

Canonical pre-commit / pre-PR check. Equivalent to the three commands
in the project's `AGENTS.md`:

```bash
./scripts/qa-pipeline.sh
```

Fails fast on the first unsuccessful step.

---

## `lab-ctao-adapter`

Compiled binary that ingests `*_internalSDC.json` files produced by the
CTA dataset pipeline and emits a `scheduling_problem.json` validated
against `schemas/scheduling_problem/scheduling_problem.schema.json`.

```bash
cargo run -p lab --bin lab-ctao-adapter -- \
    --input  data/raw/cta_n_internalSDC.json \
    --output data/cta_n.json
```

Notes:

- The output is deterministic for a fixed input.
- Re-run whenever the upstream dataset format bumps its version.

---

## `upload_results.sh`

Back-compat wrapper around `phd publish`. New scripts and humans should
prefer `cargo run -p lab --bin phd -- publish` directly. This wrapper exists
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

## `webapp`

The webapp backend. Runs the workspaces API plus the TSI adapter
endpoints. For local development:

```bash
PHD_WORKSPACES_DIR=./workspaces cargo run -p webapp --bin webapp
```

For production / Docker, see `webapp/setup.sh` and `webapp/docker/`.
