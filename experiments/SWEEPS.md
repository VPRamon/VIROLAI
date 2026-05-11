# Scheduler experiment sweeps

The canonical way to run a sweep is via `phd sweep`. It handles temp directory
management, flat output, and per-cell terminal progress automatically.

```bash
# Minimal — schedules land in ./out/
phd sweep --spec experiments/est_sweep.json

# Custom output directory
phd sweep --spec experiments/est_sweep.json --out results/my-run

# Also emit companion manifest JSONs (for phd publish)
phd sweep --spec experiments/est_sweep.json --manifest
```

`phd sweep` runs the `experiments` binary internally with `--no-state`, so no
`state.jsonl` is produced and per-cell `▶/✓/✗` progress lines are printed to
stderr.

Each output schedule JSON is self-contained and carries an embedded
`schedule_metrics` field — no separate `metrics/` directory.

---

## Direct `experiments` binary usage

The `experiments` binary is available for advanced use (custom output dirs,
resume, programmatic access):

```bash
# Run and write output under the directory declared in the spec
cargo run --manifest-path experiments/Cargo.toml -- run --spec experiments/est_sweep.json

# No state file — same behaviour as phd sweep
cargo run --manifest-path experiments/Cargo.toml -- run --spec experiments/est_sweep.json --no-state

# Resume a previous run (requires state.jsonl — incompatible with --no-state)
cargo run --manifest-path experiments/Cargo.toml -- run \
  --spec experiments/est_sweep.json \
  --resume out/my-sweep/run-<ts>
```

Output directory layout produced by `experiments run`:

```
<output_dir>/<run-timestamp>/
├── experiment.json          # spec + resolved cell list
├── state.jsonl              # per-cell completion log (omitted with --no-state)
└── schedules/
    └── <cell_id>.json       # one self-contained schedule JSON per cell
```

---

## EST sweep

`est_sweep.json` sweeps:

- `endangered_thresholds`
- `k_beams`
- `branching_factors`

## HAP sweep

`hap_sweep.json` sweeps the HAP planner configuration:

- `iota_max_values`: CRU task-scheduling iteration cap
- `rho_values`: CRU-S stochastic candidate range
- `population_sizes`: HAP multi-start population per block
- `survivor_modes`: `elitist_top_k` or `pareto_front`
- `survivor_caps`: `k`/front cap for the survivor mode
- `seeds`: deterministic master RNG seeds

## Combined paper sweep

```bash
phd sweep --spec experiments/paper_sweep.json --manifest
```

## Spec shape

```json
{
  "name": "my-sweep",
  "datasets": [
    { "id": "ctao_n", "path": "../data/ctao_n.json" }
  ],
  "output_dir": "../out/my-sweep",
  "algorithms": {
    "est": {
      "endangered_thresholds": [1, 2],
      "k_beams": [1, 4],
      "branching_factors": [1, 2]
    },
    "hap": {
      "iota_max_values": [128],
      "rho_values": [3],
      "population_sizes": [4],
      "survivor_modes": ["elitist_top_k"],
      "survivor_caps": [4],
      "seeds": [0]
    }
  }
}
```
