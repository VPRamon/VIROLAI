# LST

LST is single-backward cursor scheduling over the full horizon.

## Model

One cursor starts at the horizon end and schedules tasks as late as possible while using the same beam-search engine as EST.

Production LST follows this path:

`LstScheduler` -> `MultiCursorConfig::single_backward(...)` -> cursor engine

There is no production mirror/EST/unmirror fast path.

## Frame time and schedule time

Backward behavior is implemented with `CursorFrame::Mirrored`:

- feasibility windows are mirrored into frame time
- the queue computes earliest-feasible placements in frame time
- placements are mapped back to schedule time

An earliest-feasible choice in frame time therefore becomes a latest-start choice in schedule time.

## Configuration

LST uses the same public beam-search flags and sweep axes as EST:

- `--est-fom`
- `--est-e`
- `--est-k`
- `--est-b`

The shared flag names reflect the common cursor engine.

## CLI example

```bash
cargo run -p lab --bin virolai -- run \
  data/isdc_n.json \
  --algorithm lst \
  --est-k 4 \
  --est-b 2
```

## Sweep example

```json
{
  "kind": "lst",
  "axes": {
    "endangered_thresholds": [0, 1, 2],
    "k_beams": [1, 4],
    "branching_factors": [1, 2],
    "foms": ["soft_constraint", "future_flexibility"]
  }
}
```

## Invariants

LST inherits the cursor-engine invariants:

- a task is placed at most once
- placements do not overlap
- placements stay inside the active region
- scheduling-block dependencies are enforced

Multi-cursor forward/backward combinations are configured through experiment specifications.
