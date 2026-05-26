# AGENTS

## Project overview

This repository is a Rust scheduling project.

- Workspace crates:
  - `schedulers` library/binary in `schedulers/`
  - `lab` in `lab/` for parameter sweeps, manifests, publishing, and the top-level CLI
  - `webapp` in `webapp/` for the PhD-TSI backend integration
- Binaries:
  - `schedulers` in `schedulers/src/main.rs`
  - `lab` in `lab/src/main.rs`
  - `phd` in `lab/src/bin/phd/main.rs`
  - `lab-ctao-adapter` in `lab/src/bin/lab_ctao_adapter/main.rs`
  - `webapp` in `webapp/src/main.rs`
- Webapp integration assets under `webapp/`:
  - TSI submodule in `webapp/TSI/`
  - Adapted Docker stack in `webapp/docker/`
  - PhD adapter server sources in `webapp/src/`
- `lab/`: lab crate sources plus runnable sweep specs such as `hap_sweep.json` and `paper_sweep.json`
- `data/`: example ctao_n.json / Cctao_s.json datasets and convenience JSON files
- `schemas/`: modular JSON schemas for scheduling problems, blocks, algorithms, statistics, and schedules
- `siderust/`: local dependency crate (astronomy/time/coordinate utilities)

## Minimum QA requirements (for any task)

From the repository root, these must pass:

```bash
cargo clippy --workspace --exclude tsi-rust --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --exclude tsi-rust --all-features
```

Equivalent:

```bash
./scripts/qa-pipeline.sh
```

If formatting fails, run:

```bash
cargo fmt --all
```
