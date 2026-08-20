# Algorithm evaluation environment

This document is a legacy design note. The current experiment workflow is documented in the repository [README](../README.md), [algorithm guide](algorithms/README.md), and [sweep configuration](algorithms/sweep-configuration.md).

The historical implementation used an on-disk experiment catalog, per-cell files, an experiments REST domain, and a dedicated experiments UI. Those components are no longer the canonical workflow.

## Current workflow

```bash
cargo run --release -p lab --bin virolai -- sweep \
  --spec my-experiment.json \
  --run-db .lab/runs.sqlite

cargo run -p lab --bin lab -- registry export \
  --run-db .lab/runs.sqlite \
  --out-dir out/results

cargo run -p lab --bin virolai -- publish \
  --workspace results \
  --dir out/results
```

The current architecture stores runs in SQLite and materializes schedules only when they are exported.

## Metrics

`schedulers::metrics` remains the source of truth for schedule evaluation. Objective and descriptive metrics belong in that module rather than in dataset-specific adapters or sweep orchestration.

## Historical references

Some source paths still contain names from the previous experiments implementation, including `webapp/phd-extensions/`. Treat those as implementation history rather than project branding. New user-facing names and documentation use VIROLAI.

Likewise, older `PHD_*` environment variables may remain accepted as compatibility fallbacks. New configuration should use the corresponding `VIROLAI_*` names.

## QA

Run the repository checks with:

```bash
./scripts/qa-pipeline.sh
```
