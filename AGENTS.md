# AGENTS

## Project overview

VIROLAI (Versatile Infrastructure for Resource Optimization Leveraging Artificial Intelligence) is a Rust resource scheduling and optimization project.

- `schedulers/`: scheduling library and standalone scheduler binary
- `lab/`: experiment runner, SQLite registry, dataset adapters, and workflow CLI
- `webapp/`: result inspection and TSI integration
- `schemas/`: JSON schemas for scheduling problems, blocks, algorithms, metrics, and schedules
- `siderust/`: astronomy, time, and coordinate utilities used by current integrations

Main binaries:

- `schedulers` in `schedulers/src/main.rs`
- `lab` in `lab/src/main.rs`
- `virolai` in `lab/src/bin/virolai/main.rs`
- `lab-ctao-adapter` in `lab/src/bin/lab_ctao_adapter/main.rs`
- `webapp` in `webapp/src/main.rs`

The scheduler consumes the generic `scheduling_problem.json` model. CTAO support is an optional dataset adapter and evaluation integration; do not introduce CTAO-specific assumptions into scheduler architecture or algorithm APIs.

Historical source paths under `webapp/phd-extensions/` and `webapp/src/phd_tsi_adapter.rs` remain for compatibility. New user-facing branding should use VIROLAI.

## Minimum QA requirements

From the repository root, these commands must pass:

```bash
cargo clippy --workspace --exclude tsi-rust --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --exclude tsi-rust --all-features
```

Equivalent command:

```bash
./scripts/qa-pipeline.sh
```

If formatting fails:

```bash
cargo fmt --all
```
