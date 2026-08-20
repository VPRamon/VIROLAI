# EST

EST is single-forward cursor scheduling over the full horizon.

## Model

One cursor starts at the horizon start and moves forward. At each step, it evaluates earliest feasible placements with endangered-task protection and beam pruning.

Production EST follows this path:

`EstScheduler` -> `MultiCursorConfig::single_forward(...)` -> cursor engine

`EstScheduler` remains the public API. Beam-search execution is implemented by the shared cursor engine.

## Candidate ordering

EST ordering is deterministic:

1. schedulable before impossible
2. effective earliest start
3. endangered promotion
4. original EST
5. lower flexibility first
6. higher soft priority
7. lower task id

The endangered rule delays a non-endangered task when placing it would block an endangered task whose EST falls inside the occupied interval.

## Configuration

`endangered_threshold` uses residual flexibility. A value of `0` disables endangered protection.

`k_beams` controls the number of partial schedules retained after each round.

`branching_factor` controls the number of candidate actions explored from each beam state.

`fom` selects the beam ranking function. Current options include `soft_constraint` and `future_flexibility`.

See [figures-of-merit.md](figures-of-merit.md).

## CLI example

```bash
cargo run -p lab --bin virolai -- run \
  data/isdc_n.json \
  --algorithm est \
  --est-k 4 \
  --est-b 2
```

## Sweep example

```json
{
  "kind": "est",
  "axes": {
    "endangered_thresholds": [0, 1, 2],
    "k_beams": [1, 4],
    "branching_factors": [1, 2],
    "foms": ["soft_constraint", "future_flexibility"]
  }
}
```

## Invariants

EST inherits the cursor-engine invariants:

- a task is placed at most once
- placements do not overlap
- placements stay inside the active region
- scheduling-block dependencies are enforced

Use `multi_cursor` when several coordinated scheduling frontiers are required.
