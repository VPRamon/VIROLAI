# EST

EST is **single-forward cursor scheduling over the full horizon**.

## Mental model

There is one cursor starting at the horizon start and moving forward. At each step it tries the earliest feasible placements first, with endangered-task protection and beam pruning.

## Implementation path

Production EST is implemented as:

`EstScheduler` → `MultiCursorConfig::single_forward(...)` → cursor engine

`EstScheduler` remains the public API, but the beam-search execution lives in the shared cursor engine.

## Candidate ordering

EST ordering is deterministic:

1. schedulable before impossible
2. effective earliest start
3. endangered promotion
4. original EST
5. lower flexibility first
6. higher soft priority
7. lower task id

The endangered-promotion rule delays a non-endangered task when it would block an endangered task whose EST falls inside its occupied interval.

## Configuration knobs

### `endangered_threshold`

This threshold is based on **residual flexibility**, not on a count of scheduling blocks.

- `0` disables endangered protection
- a task is endangered when `flexibility < endangered_threshold`

### `k_beams`

Beam width: how many partial schedules survive each round.

### `branching_factor`

How many candidate actions are explored from one beam state in a round.

### `fom`

Beam ranking function. Current user-facing options are:

- `soft_constraint`
- `future_flexibility`

See [figures-of-merit.md](figures-of-merit.md).

## CLI example

```bash
cargo run -p lab --bin phd -- run data/isdc_n.json --algorithm est --est-k 4 --est-b 2
```

## Sweep-spec example

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

- no duplicate task
- no overlap
- placements stay in the active region
- scheduling-block dependencies are enforced

## Limitations

- EST is single-frontier by design; use `multi_cursor` when you want several coordinated frontiers.
- The raw standalone CLI exposes EST directly, but not arbitrary multi-cursor layouts.

## Relevant tests

- `est_wrapper_matches_multicursor_single_forward_basic`
- `est_wrapper_matches_multicursor_single_forward_endangered`
- `est_wrapper_matches_multicursor_single_forward_beam`
- `est_wrapper_matches_multicursor_single_forward_soft_constraints`
- `single_forward_constructor_matches_est`
