# LST

LST is **single-backward cursor scheduling over the full horizon**.

## Mental model

There is one cursor anchored at the horizon end. It schedules tasks as late as possible while still using the same beam-search machinery as EST.

## Implementation path

Production LST is implemented as:

`LstScheduler` → `MultiCursorConfig::single_backward(...)` → cursor engine

There is **no** production `mirror → EST → unmirror` fast path anymore.

## Frame time vs schedule time

The cursor engine handles backward behavior with `CursorFrame::Mirrored`:

- feasibility windows are mirrored into frame time
- the queue still computes earliest-feasible placements in frame time
- those placements are mapped back into schedule time

So an EST-style choice in frame time becomes a latest-start choice in schedule time.

## Configuration knobs

LST uses the same public flags and sweep axes as EST:

- `--est-fom`
- `--est-e`
- `--est-k`
- `--est-b`

That shared flag naming reflects the shared beam-search machinery, not a hidden call from LST into a separate EST engine.

## CLI example

```bash
cargo run -p lab --bin phd -- run data/isdc_n.json --algorithm lst --est-k 4 --est-b 2
```

## Sweep-spec example

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

## Compatibility note

`MirroredFom` still exists only as a crate-internal compatibility/test helper. It is not part of the public API and not used by the production LST scheduler path.

## Invariants

LST inherits the cursor-engine invariants:

- no duplicate task
- no overlap
- placement must stay in the active region
- dependencies remain enforced

## Limitations

- Raw `schedulers` / `phd run` expose LST only as a single-run algorithm.
- Multi-cursor backward/forward hybrids are configured through `lab run` / `phd sweep` specs.

## Relevant tests

- `lst_wrapper_matches_multicursor_single_backward_basic`
- `lst_wrapper_matches_multicursor_single_backward_endangered`
- `lst_wrapper_matches_multicursor_single_backward_beam`
- `lst_wrapper_matches_multicursor_single_backward_soft_constraints`
- `single_backward_constructor_matches_lst`
