# Scheduler experiment sweeps

The experiment runner is the `est_experiment` binary. It preschedules the input
once, runs every configured scheduler variant, and writes a timestamped output
directory containing:

- `manifest.json`
- `comparison.csv`
- `schedules/*.json`
- `schedules/*.est_trace.jsonl` for EST runs when traces are enabled

Run commands from the repository root.

## EST sweep

```bash
cargo run --bin est_experiment -- --spec experiments/est_sweep.json
```

`experiments/est_sweep.json` sweeps:

- `endangered_thresholds`
- `k_beams`
- `branching_factors`

You can override EST axes from the CLI:

```bash
cargo run --bin est_experiment -- data/ctao_n.json \
  --output-dir out/est \
  --est-e-values 1,2,4 \
  --est-k-values 1,4,8 \
  --est-b-values 1,2,4
```

## HAP sweep

```bash
cargo run --bin est_experiment -- --spec experiments/hap_sweep.json
```

`experiments/hap_sweep.json` sweeps the HAP planner configuration:

- `iota_max_values`: CRU task-scheduling iteration cap
- `rho_values`: CRU-S stochastic candidate range
- `population_sizes`: HAP multi-start population per block
- `survivor_modes`: `elitist_top_k` or `pareto_front`
- `survivor_caps`: `k`/front cap for the survivor mode
- `seeds`: deterministic master RNG seeds

## Combined paper sweep

```bash
cargo run --bin est_experiment -- --spec experiments/paper_sweep.json
```

This runs both the EST and HAP sweeps against `data/ctao_n.json` and writes the
combined comparison under `out/`.

## Spec shape

New specs use per-algorithm sweep blocks:

```json
{
  "input_json": "../data/ctao_n.json",
  "output_dir": "../out/",
  "emit_trace": true,
  "sweep": {
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

If `emit_trace` is true, only EST runs emit trace JSONL files; HAP runs currently
write schedule JSON and comparison metrics only.
