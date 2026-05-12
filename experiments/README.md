# `experiments` crate

The `experiments` crate runs parameter-sweep experiments against the
`scheduler` library. Its only CLI entrypoint is `experiments run`,
which expands an `ExperimentSpec` into a matrix of cells and executes
them in parallel.

Most users should still drive experiments through `phd sweep`, but this
crate is the source of truth for the spec format, raw output layout,
checkpointing, and resume semantics.

## Entry points

### Recommended: `phd sweep`

```bash
cargo run --bin phd -- sweep \
    --spec experiments/hap_sweep.json \
    --out out/hap-sweep \
    --manifest
```

`phd sweep` invokes the sibling `experiments` binary, then flattens the
raw `run-<timestamp>/schedules/*.json` tree into:

```text
out/hap-sweep/
    <cell_id>.json
    <cell_id>.manifest.json
```

This flat layout is the canonical input for `phd publish`.

`phd sweep` expects the `experiments` binary at
`target/debug/experiments` or `target/release/experiments`. Build it
explicitly when needed:

```bash
cargo build --manifest-path experiments/Cargo.toml --target-dir target
cargo build --release --manifest-path experiments/Cargo.toml --target-dir target
```

### Direct crate invocation: `experiments run`

```bash
cargo run --manifest-path experiments/Cargo.toml -- run \
    --spec experiments/hap_sweep.json
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
- `schedule_metrics` for downstream ranking and comparison

There are no separate `metrics/` or `traces/` directories in this
workflow.

## Spec format

The JSON schema is defined by `experiments::spec::ExperimentSpec`.
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
  "ranking": { "completion": 2.0, "priority": 1.0 },
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
| `ranking` | Optional weights mirrored into metrics output |
| `max_parallel` | Optional concurrency cap |
| `output_dir` | Root directory for raw run artifacts |

Working examples live next to this README:

- `experiments/hap_sweep.json`
- `experiments/paper_sweep.json`

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
cargo run --bin phd -- publish \
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
- Do not use `cargo run -p experiments`; this crate is not a workspace member.
