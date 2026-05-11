# Scheduler experiment sweeps

The experiment runner is the `experiments` binary (in this crate). It
preschedules the input once, runs every configured scheduler variant, and
writes a timestamped output directory containing:

- `experiment.json`
- `schedules/*.json`
- `state.jsonl` — per-cell completion log
- `schedules/*.est_trace.jsonl` for EST runs when traces are enabled

Run commands from the repository root.

## EST sweep

```bash
cargo run --manifest-path experiments/Cargo.toml -- run --spec experiments/est_sweep.json
# or via the phd dispatcher:
phd matrix run --spec experiments/est_sweep.json
```

`est_sweep.json` sweeps:

- `endangered_thresholds`
- `k_beams`
- `branching_factors`

## HAP sweep

```bash
cargo run --manifest-path experiments/Cargo.toml -- run --spec experiments/hap_sweep.json
```

`hap_sweep.json` sweeps the HAP planner configuration:

- `iota_max_values`: CRU task-scheduling iteration cap
- `rho_values`: CRU-S stochastic candidate range
- `population_sizes`: HAP multi-start population per block
- `survivor_modes`: `elitist_top_k` or `pareto_front`
- `survivor_caps`: `k`/front cap for the survivor mode
- `seeds`: deterministic master RNG seeds

## Combined paper sweep

```bash
cargo run --manifest-path experiments/Cargo.toml -- run --spec experiments/paper_sweep.json
```

This runs both the EST and HAP sweeps and writes the combined output under `out/`.

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
