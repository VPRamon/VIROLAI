# AGENTS

## Project overview

This repository is a Rust scheduling project.

- Crate: `scheduler` (library) in `src/`
- Binaries:
  - `scheduler` in `src/main.rs` (entry point)
  - `ctao_adapter` in `scripts/ctao_adapter.rs`: converts CTA dataset files (`*_internalSDC.json`) into a `scheduling_problem.json` payload validated by `schemas/scheduling_problem/scheduling_problem.schema.json`
- Webapp integration assets under `webapp/`:
  - TSI submodule in `webapp/TSI/`
  - Adapted Docker stack in `webapp/docker/`
  - PhD adapter server sources in `webapp/scripts/` (`phd_tsi_server.rs`, `phd_tsi_adapter.rs`)
- `data/`: example ctao_n.json / Cctao_s.json datasets and convenience JSON files
- `schemas/`: modular JSON schemas for scheduling problems, blocks, algorithms, statistics, and schedules
- `siderust/`: local dependency crate (astronomy/time/coordinate utilities)

## Minimum QA requirements (for any task)

From the repository root, these must pass:

```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --all-features
```

Equivalent:

```bash
./scripts/qa-pipeline.sh
```

If formatting fails, run:

```bash
cargo fmt --all
```
