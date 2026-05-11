# Sweeps — operational guide

A **sweep** is a single invocation of the experiment runner over a
matrix of cells. This document covers the operational details: how to
size sweeps, how to resume them, and how to publish their output.

For the conceptual pipeline (sweep → publish → workspace), see
`docs/architecture.md`. For the runner internals, see
`experiments/README.md`.

---

## 1. Plan the sweep

A spec is a matrix of `datasets × algorithms × seeds × params`. Pick:

- **`max_parallel`** — concurrent cells. Match to physical cores.
- **`seeds`** — repetitions per `(dataset, algorithm)` pair. Use ≥ 3
  for any statistic you intend to publish.
- **Algorithm `kind`s** — see `scheduler::algorithms` for the full list.

Templates live in `experiments/hap_sweep.json` and
`experiments/paper_sweep.json`.

---

## 2. Run

Recommended (flat output, manifests on the side):

```bash
cargo run --release --bin phd -- sweep \
    --spec experiments/paper_sweep.json \
    --out  out/paper-sweep \
    --manifest
```

This produces:

```text
out/paper-sweep/
    <cell_id>.json           # self-contained schedule (metrics embedded)
    <cell_id>.manifest.json  # lightweight manifest referencing the schedule
```

Cell ids are deterministic: `<dataset>__<algorithm>__seed<N>__<params-hash>`.

---

## 3. Resume

`phd sweep` checkpoints by default. Re-running with the same `--out`
and `--spec` skips cells already present (matched by `cell_id`). To
force a full re-run, delete the relevant `<cell_id>.json` files (or
`--out` entirely).

For checkpoint-free runs (faster IO, no resume), invoke the runner
directly with `--no-state`:

```bash
cargo run -p experiments -- run --spec … --out … --no-state
```

---

## 4. Publish

A single command publishes the whole sweep:

```bash
cargo run --release --bin phd -- publish \
    --workspace paper-2024 \
    --dir       out/paper-sweep \
    --create-workspace \
    --include-schedules
```

Behaviour:

- Every `*.manifest.json` is POSTed to `…/manifests/batch`.
- Every other `.json` is parsed; if it carries `schedule_metadata` and
  `schedule_metrics` it is POSTed to `…/schedules/batch` (the server
  derives a manifest and persists the full schedule for drill-down).
- `--include-schedules false` skips the schedule batch entirely
  (smaller payload, no drill-down later).
- Idempotency is automatic: manifests deduplicate on `manifest_id`,
  schedules on content SHA-256.

---

## 5. Sizing & performance tips

- Use `--release` for any sweep > a few minutes wall time.
- `max_parallel` saturates first; tune `seeds` second.
- `out/` lives on the local disk by default — sweeps with > 10 k cells
  produce > 1 GB. Either prune intermediate cells or publish to the
  workspace and rely on its content-addressed dedupe.
- The webapp's `/workspaces/<id>/comparison` endpoint reads only
  manifests, so even very large sweeps stay browsable.

---

## 6. Anti-patterns

- **Do not** edit individual cell JSONs by hand — their
  `schedule_metrics` block is the source of truth for any downstream
  comparison.
- **Do not** maintain parallel `metrics/` or `traces/` directories.
  That layout is gone; metrics live inside the schedule.
- **Do not** roll your own HTTP client to upload results — `phd
  publish` (or the `upload_results.sh` wrapper) is the supported path.
