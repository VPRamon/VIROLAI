# Cursor Engine

The cursor engine (`schedulers::scheduler::cursor`) is the single production beam-search engine for **EST**, **LST**, and all **multi-cursor** layouts.

It does not special-case:

- EST vs LST
- fixed vs dynamic territories
- one cursor vs many cursors

Those differences are expressed entirely through configuration.

## Core abstractions

### Cursor

A cursor is one search frontier with:

- a stable `id`
- a `direction` (`Forward` or `Backward`)
- a `territory`
- a `frame`
- a candidate queue

Each cursor proposes placements inside its own current **active region**.

### Territory

A cursor territory is the schedule-time region it is allowed to use.

- **Static Partitioning**: absolute or fractional `[start, end)` region
- **Dynamic Frontiering**: one or both boundaries follow another cursor's live position via `BoundaryRef::{HorizonStart, HorizonEnd, Cursor(id)}`

Dynamic territories may also enforce a `min_gap`.

### Active region

The active region is the concrete schedule-time interval a cursor may place into **this round**. It is recomputed from the territory and the live cursor snapshot before every beam expansion.

### Frame

Every cursor runs forward in its own local **frame time**:

- `CursorFrame::Identity` for forward cursors
- `CursorFrame::Mirrored` for backward cursors

This is why the engine can stay direction-agnostic in its hot path. Candidate queues always compute earliest-feasible placements in frame time.

## Frame time vs schedule time

Backward behavior is not implemented by a separate scheduler. Instead:

- the backward cursor uses `CursorFrame::Mirrored`
- feasibility windows are mirrored into frame time
- the queue still chooses earliest-feasible frame-time placements
- the chosen placement is mapped back into schedule time

So **earliest in frame time == latest in schedule time**.

That is how `LstScheduler` is implemented today: not by external mirroring, but by a single backward cursor running through the same engine.

## Beam-search loop

Each round:

1. Snapshot live cursor frontiers into a `CursorWorld`
2. Resolve each cursor's active region
3. Refresh and sort every cursor queue against that live region
4. Rank possible cursor actions globally
5. Build child states by placing one candidate
6. Validate the placement
7. Recompute per-cursor active regions for the child context
8. Score the child with the chosen FOM
9. Keep the top `k_beams` children across all expansions

The engine uses the same EST-style candidate ordering and the same global top-`k` pruning whether there is one cursor or several.

## Placement validity invariants

The engine enforces schedule validity independently of scoring:

- no duplicate task placement
- no overlap
- placement must stay inside the cursor's active region
- scheduling-block dependencies must be respected

The FOM can only rank valid child states. It never authorizes an invalid placement.

## Action ranking

When several cursors can act in the same round, action ordering is deterministic:

1. candidate rank inside that cursor's queue
2. cursor id tie-break
3. task id tie-break

Inside one cursor, candidate ordering is inherited from the EST ordering helpers:

1. schedulable before impossible
2. effective earliest start
3. endangered before non-endangered at the same effective start
4. original EST
5. lower flexibility first
6. higher soft priority
7. lower task id

## Territory Models

### Static Partitioning

The territory is static for the whole run.

Examples:

- `est_lst_split`
- `start_mid_forward`
- `four_quarter_forward`

These layouts split the horizon at fixed boundaries.

### Dynamic Frontiering

The territory boundary may depend on another cursor's live position. The engine:

- snapshots live frontiers
- resolves boundaries against that snapshot
- recomputes active regions every round

This lets cursors meet or constrain each other dynamically without changing the beam-search core.

Examples:

- `dynamic_est_lst_meet`
- `dynamic_start_mid_forward`

## Dynamic boundaries and `min_gap`

Dynamic boundaries use live cursor positions from the current beam state. If a boundary references another cursor:

- left boundary uses that cursor's position plus `min_gap`
- right boundary uses that cursor's position minus `min_gap`

If the resulting region becomes empty or crossed, the cursor simply has no active region that round.

## Why there is only one engine

The production scheduler path is now:

- EST wrapper → cursor engine
- LST wrapper → cursor engine
- multi-cursor layouts → cursor engine

No production code calls a separate EST beam engine, and no production code mirrors LST externally before delegating to EST.

## Relevant tests

- EST wrapper equivalence:
  - `est_wrapper_matches_multicursor_single_forward_basic`
  - `est_wrapper_matches_multicursor_single_forward_endangered`
  - `est_wrapper_matches_multicursor_single_forward_beam`
  - `est_wrapper_matches_multicursor_single_forward_soft_constraints`
- LST wrapper equivalence:
  - `lst_wrapper_matches_multicursor_single_backward_basic`
  - `lst_wrapper_matches_multicursor_single_backward_endangered`
  - `lst_wrapper_matches_multicursor_single_backward_beam`
  - `lst_wrapper_matches_multicursor_single_backward_soft_constraints`
- Dynamic boundary behavior:
  - `dynamic_boundary_to_cursor_position_updates_after_placement`
  - `dynamic_min_gap_keeps_cursors_apart`
  - `dynamic_est_lst_cursors_move_until_meeting`
  - `dynamic_start_mid_forward_front_respects_mid_cursor`
