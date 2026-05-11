# `experiments` crate

Multi-cell experiment **runner** used to sweep a scheduler across a
matrix of `(dataset × algorithm × seed × …)` cells. This crate provides
the engine; researchers normally drive it through the higher-level
`phd sweep` subcommand of the main `scheduler` binary.

The output of every run is a set of **self-contained schedule JSONs**
(metrics embedded) plus a small `experiment.json` describing the
matrix. There are no separate `metrics/` or `traces/` sub-directories;
that legacy layout is gone.

---

## Two ways to drive the runner

### A. `phd sweep` (recommended)

```bash
cargo run --bin phd -- sweep \
    --spec experiments/hap_sweep.json \
    --out  out/hap-sweep \
    --manifest
```

`phd sweep` invokes this crate under the hood, then flattens the
`run-<timestamp>/schedules/*.json` tree into `out/hap-sweep/<cell>.json`
and (with `--manifest`) emits a sibling `out/hap-sweep/<cell>.manifest.json`
for each cell. This is the canonical layout consumed by `phd publish`.

### B. `experiments run` (advanced / direct)

```bash
cargo run -p experiments -- run \
    --spec experiments/hap_sweep.json \
    --out  out/hap-sweep-raw
```

Direct invocation preserves the full per-run layout described below.
Use this when you need checkpoints, custom resume semantics, or want to
inspect the unflattened output.

---

## Output layout (direct invocation)

```text
<out>/<experiment_slug>/run-<timestamp>/
    experiment.json     # resolved spec + complete cell list
    schedules/
        <cell_id>.json  # self-contained schedule, embeds schedule_metrics
    state.jsonl         # append-only checkpoint stream (omitted with --no-state)
```

Each `schedules/<cell_id>.json` matches `schemas/schedule/...` and
includes a `schedule_metadata` block (dataset, algorithm, seed, params,
horizon) and a `schedule_metrics` block (`schemas/scheduling_statistics/
schedule_metrics.schema.json`). This is the **single source of truth**
for the cell — manifests are derived from it.

---

## Spec format (matrix)

```jsonc
{
    "name": "hap_sweep",
    "output_dir": "out",
    "max_parallel": 4,
    "datasets": [
        { "id": "lst_sh", "path": "data/lst_sh.json" }
    ],
    "algorithms": [
        {
            "id": "hap_i64",
            "kind": "Hap",
            "params": { "iterations": 64 }
        }
    ],
    "seeds": [1, 2, 3]
}
```

The full schema is enforced by `experiments::spec::ExperimentSpec`. See
`experiments/hap_sweep.json` and `experiments/paper_sweep.json` for
working examples.

---

## Resume / checkpoints

Each cell append-writes to `state.jsonl` as it completes. Re-running
with the same `--out` and `--name` skips cells already marked done. To
disable checkpoints (smaller IO, no resume), pass `--no-state`.

---

## Publishing results

After a sweep finishes, publish the entire output directory in one
command:

```bash
cargo run --bin phd -- publish \
    --workspace paper \
    --dir       out/hap-sweep \
    --create-workspace \
    --include-schedules
```

`phd publish` walks `--dir`, classifies each `.json` as a manifest or
a self-contained schedule, and uploads them in chunked batches against
the workspaces backend. Idempotency is automatic. See
`docs/architecture.md` for the full pipeline.
