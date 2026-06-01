# HAP

HAP is the repository's **hybrid adaptive/metaheuristic planner**. Unlike EST/LST/multi-cursor, it is **not** implemented on top of the cursor engine.

## Mental model

HAP builds schedules through planner rounds and CRU-based repair/search rather than through a cursor-frontier beam search.

At the lab/sweep level, the resolved run configuration is:

- `iota_max`
- `rho`
- `population_size`
- `survivor_mode`
- `survivor_cap`
- `seed`

## Planner configuration

### `iota_max`

Maximum CRU task-scheduling iterations.

### `rho`

Stochastic candidate-window range used by the CRU-S selector.

### `population_size`

Number of candidate schedules explored per block at the planner level.

### `survivor_mode`

How schedules survive into the next planner round:

- `greedy_one`
- `elitist_top_k`
- `pareto_front`

### `survivor_cap`

Cap applied to survivor modes that need one.

### `seed`

Deterministic master RNG seed. Reusing the same full HAP config, including `seed`, reproduces the same stochastic decisions.

## How HAP differs from cursor-engine algorithms

| Cursor-engine family | HAP |
|---|---|
| Beam search | Planner / CRU search |
| One or more cursor frontiers | Population of candidate schedules |
| EST ordering helpers | CRU selector + planner survivor logic |
| `soft_constraint` / `future_flexibility` rank beam states | HAP uses its own planner fitness logic |

## CLI example

The raw single-run CLI exposes the simpler HAP knobs:

```bash
cargo run -p lab --bin phd -- run data/isdc_n.json --algorithm hap \
  --hap-num-crus 8 --hap-cru-iterations 128 --hap-rho 3 --hap-seed 42
```

For sweeps, use the richer experiment-spec shape below.

## Sweep-spec example

```json
{
  "kind": "hap",
  "axes": {
    "iota_max_values": [64, 128],
    "rho_values": [3, 5],
    "population_sizes": [4, 8],
    "survivor_modes": ["elitist_top_k", "pareto_front"],
    "survivor_caps": [4],
    "seeds": [0, 1, 2]
  }
}
```

## Invariants

HAP still produces ordinary validated schedules. Its search policy differs from the cursor engine, but schedule validity remains independent of ranking heuristics.

## Limitations

- The raw standalone CLI exposes a narrower HAP configuration surface than the sweep runner.
- Multi-cursor docs do not apply to HAP because it does not use the cursor engine.
