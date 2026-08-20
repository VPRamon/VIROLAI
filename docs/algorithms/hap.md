# HAP

HAP is VIROLAI's adaptive/metaheuristic planner. It is separate from the cursor engine used by EST, LST, and multi-cursor scheduling.

## Model

HAP builds schedules through planner rounds and CRU-based repair/search rather than cursor-frontier beam search.

The resolved experiment configuration includes:

- `iota_max`
- `rho`
- `population_size`
- `survivor_mode`
- `survivor_cap`
- `seed`

`iota_max` limits CRU task-scheduling iterations. `rho` controls the stochastic candidate-window range. `population_size` controls how many candidate schedules are explored. `survivor_mode` determines which schedules continue to the next round, and `survivor_cap` limits the survivor set. `seed` provides deterministic stochastic runs.

## CLI example

```bash
cargo run -p lab --bin virolai -- run \
  data/isdc_n.json \
  --algorithm hap \
  --hap-num-crus 8 \
  --hap-cru-iterations 128 \
  --hap-rho 3 \
  --hap-seed 42
```

## Sweep example

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

## Relation to cursor-engine algorithms

| Cursor-engine family | HAP |
| --- | --- |
| Beam search | Planner and CRU search |
| One or more cursor frontiers | Population of candidate schedules |
| Cursor candidate ordering | CRU selector and survivor logic |
| Beam-state FOMs | HAP planner fitness logic |

HAP produces the same validated schedule model as the other algorithms. Its search policy is independent of schedule validity.
