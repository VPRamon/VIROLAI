# `lab` crate

The `lab` crate runs parameter-sweep experiments against the
`scheduler` library. Its only CLI entrypoint is `lab run`,
which expands an `ExperimentSpec` into a matrix of cells and executes
them in parallel.

Most users should still drive experiments through `phd sweep`, but this
crate is the source of truth for the spec format, raw output layout,
checkpointing, and resume semantics.

## Entry points

### Recommended: `phd sweep`

```bash
cargo run -p lab --bin phd -- sweep \
    --spec lab/hap_sweep.json \
    --out out/hap-sweep \
    --manifest
```

`phd sweep` invokes the sibling `lab` binary, then flattens the
raw `run-<timestamp>/schedules/*.json` tree into:

```text
out/hap-sweep/
    <cell_id>.json
    <cell_id>.manifest.json
```

This flat layout is the canonical input for `phd publish`.

`phd sweep` expects the `lab` binary at
`target/debug/lab` or `target/release/lab`. Build it
explicitly when needed:

```bash
cargo build -p lab --bin lab
cargo build --release -p lab --bin lab
```

### Direct crate invocation: `lab run`

```bash
cargo run -p lab --bin lab -- run \
    --spec lab/hap_sweep.json
```

Supported flags:

| Flag | Description |
| --- | --- |
| `--spec <FILE>` | Required experiment spec JSON |
| `--resume <DIR>` | Reuse an existing `run-<timestamp>/` directory and skip completed cells |
| `--output-dir <DIR>` | Override `spec.output_dir` for `--dry-run` |
| `--dry-run` | Resolve cells and write `experiment.json` without executing runs |
| `--no-state` | Do not write `state.jsonl`; incompatible with `--resume` |

Use direct invocation when you need the raw run directory, checkpoint
stream, or explicit resume control.

## Output layout

Raw crate output:

```text
<output_dir>/<experiment_slug>/run-<timestamp>/
    experiment.json
    schedules/
        <cell_id>.json
    state.jsonl
```

`state.jsonl` is omitted with `--no-state`.

Each schedule JSON is self-contained and embeds:

- `schedule_metadata` for dataset, algorithm, resolved parameters, and horizon
- `schedule_metrics` with objective/descriptive run metrics for downstream
  query-time ranking and comparison

There are no separate `metrics/` or `traces/` directories in this
workflow.

## Spec format

The JSON schema is defined by `lab::spec::ExperimentSpec`.
Paths in `datasets[*].path` and `output_dir` may be relative to the
spec file.

Minimal shape:

```json
{
  "name": "paper-sweep",
  "datasets": [
    { "id": "ctao_n", "path": "../data/ctao_n.json" }
  ],
  "algorithms": [
    { "kind": "est", "axes": { "k_beams": [1, 4], "branching_factors": [1, 2] } },
    { "kind": "hap", "axes": { "iota_max_values": [64, 128], "seeds": [0, 1] } }
  ],
  "max_parallel": 4,
  "output_dir": "../out/paper"
}
```

Key fields:

| Field | Meaning |
| --- | --- |
| `name` | Human-readable experiment name; used to derive the output slug |
| `datasets` | Input scheduling problems to evaluate |
| `algorithms` | One or more `est` / `hap` sweep blocks |
| `max_parallel` | Optional concurrency cap |
| `output_dir` | Root directory for raw run artifacts |

`ranking` is accepted only as a legacy field. `lab run` records objective
metrics; scientific interpretation lives in registry/query commands such as
`lab registry sort`, `lab registry rank`, and `lab registry pareto`.

Working examples live next to this README:

- `lab/hap_sweep.json`
- `lab/paper_sweep.json`

## Sweep operations

### Planning

A sweep is the Cartesian product of `datasets × algorithms × axes`. Tune:

- `max_parallel` to match available CPU
- `seeds` high enough for the statistics you intend to compare
- axis breadth carefully, because output size grows linearly with cell count

### Resume and checkpoints

When `state.jsonl` is enabled, re-running with `--resume <run-dir>`
skips cells already recorded as completed. For checkpoint-free runs with
lower IO, use `--no-state` and accept that resume is unavailable.

### Dry runs

Use `--dry-run` to validate a spec, resolve the full cell list, and
write `experiment.json` without running the scheduler.

## Publishing

After a sweep completes, publish the flattened output with `phd
publish`:

```bash
cargo run -p lab --bin phd -- publish \
    --workspace paper \
    --dir out/hap-sweep \
    --create-workspace \
    --include-schedules
```

`phd publish` uploads manifests and schedules in batches and handles
idempotency. See `docs/architecture.md` for the end-to-end pipeline.

## Anti-patterns

- Do not edit generated cell JSONs by hand; they are the source of truth.
- Do not maintain parallel `metrics/` or `traces/` directories for this flow.
- Do not use the old `experiments/` path; this crate is now the `lab` workspace member.



### Cheatsheet


## Mejor `priority_density`

```bash
cargo run -p lab --bin lab -- registry sort \
  --run-db .lab/runs.sqlite \
  --dataset isdc_n \
  --sort priority_density:desc \
  --limit 1
```

Si quieres solo EST o LST:

```bash
cargo run -p lab --bin lab -- registry sort \
  --run-db .lab/runs.sqlite \
  --dataset isdc_n \
  --algorithm lst \
  --sort priority_density:desc \
  --limit 1
```

## Mayor fitness sum / priority sum

En el código la métrica soportada se llama:

```text
scheduled_priority_sum
```

`metric_value()` la reconoce explícitamente junto con `priority_density`, `scheduled_task_ratio`, `scheduled_priority_ratio`, etc. 

Ejecuta:

```bash
cargo run -p lab --bin lab -- registry sort \
  --run-db .lab/runs.sqlite \
  --dataset isdc_n \
  --sort scheduled_priority_sum:desc \
  --limit 1
```

Filtrando por algoritmo:

```bash
cargo run -p lab --bin lab -- registry sort \
  --run-db .lab/runs.sqlite \
  --dataset isdc_n \
  --algorithm est \
  --sort scheduled_priority_sum:desc \
  --limit 1
```

## Ver el resultado completo

Cuando tengas el `run_key` prefix que aparece en la tabla:

```bash
cargo run -p lab --bin lab -- registry inspect \
  --run-db .lab/runs.sqlite \
  --run <run_key_prefix>
```

## Regenerar el schedule ganador

```bash
cargo run -p lab --bin lab -- registry regenerate \
  --run-db .lab/runs.sqlite \
  --run <run_key_prefix> \
  --out out/best-density.json
```

```bash
cargo run -p schedulers --bin schedulers -- \
  data/isdc_n.json \
  --algorithm lst \
  --est-e 1 \
  --est-k 4 \
  --est-b 2 \
  --est-fom future_flexibility \
  --output out/schedule-lst.json
```
