# Multi-cursor

Multi-cursor scheduling runs **several cursors over one shared global schedule**.

## Mental model

Each cursor has its own:

- `id`
- direction
- territory
- frame
- candidate queue

All cursors compete to extend the same schedule. When one cursor places a task, that task is removed from every other cursor queue.

## Implementation path

Production multi-cursor scheduling always goes through:

`MultiCursorScheduler` → `engine::run_multi_cursor(...)`

The same engine also powers EST and LST wrappers.

## Fixed layouts (Plan A)

### `est_lst_split`

- cursor `0`: forward over the first half
- cursor `1`: backward over the second half

This is the simplest forward/backward split with a fixed midpoint.

### `start_mid_forward`

- cursor `0`: forward over the first half
- cursor `1`: forward over the second half

Useful when you want two forward frontiers with a static split.

## Dynamic layouts (Plan B)

### `dynamic_est_lst_meet`

- cursor `0`: forward from the horizon start
- cursor `1`: backward from the horizon end
- each cursor's inner boundary follows the other cursor's live position

The shared region shrinks as the two cursors advance. “No crossing” means one cursor may never place past the current live boundary defined by the other.

### `dynamic_start_mid_forward`

- cursor `0`: forward from the horizon start
- cursor `1`: forward from the midpoint to the horizon end
- cursor `0`'s end boundary follows cursor `1`'s live position

This prevents the front cursor from invading the middle cursor's live region while still letting the middle cursor continue to the horizon end.

## Active-region behavior

Every round the engine recomputes each cursor's active region from:

- the layout definition
- the live cursor snapshot
- the cursor direction
- any `min_gap`

If a region becomes empty, that cursor contributes no action that round.

## Configuration knobs

Multi-cursor uses EST-style beam knobs plus a layout selector:

- `layouts`
- `endangered_thresholds`
- `k_beams`
- `branching_factors`
- `foms`

## Sweep-spec example

```json
{
  "kind": "multi_cursor",
  "axes": {
    "layouts": ["est_lst_split", "dynamic_est_lst_meet"],
    "endangered_thresholds": [1],
    "k_beams": [4],
    "branching_factors": [2],
    "foms": ["soft_constraint", "future_flexibility"]
  }
}
```

## Slug naming

Multi-cursor cell slugs encode the layout first:

- `est_lst_split-e1-k4-b2`
- `dynamic_est_lst_meet-e1-k4-b2`

Non-default FOMs append a suffix such as `-future_flexibility`.

## Availability

Multi-cursor is exposed through **experiment specs** (`lab run` / `phd sweep`), not through the raw standalone scheduler CLI.

## Invariants

- one global schedule shared by all cursors
- no duplicate task placement across cursors
- no overlap
- each placement must stay inside the acting cursor's active region
- block dependencies are enforced across cursors too

## Relevant tests

Fixed-territory tests:

- `multi_cursor_two_forward_fixed_territories_no_overlap`
- `multi_cursor_forward_backward_fixed_territories_no_overlap`
- `multi_cursor_forward_backward_both_cursors_contribute`
- `multi_cursor_rejects_cross_territory_placement`
- `multi_cursor_does_not_duplicate_task_across_cursors`
- `multi_cursor_respects_block_dependencies`

Dynamic-territory tests:

- `dynamic_est_lst_cursors_move_until_meeting`
- `dynamic_est_lst_never_crosses`
- `dynamic_est_lst_no_overlap`
- `dynamic_est_lst_both_cursors_contribute`
- `dynamic_est_lst_exhausted_cursor_does_not_stop_other_cursor`
- `dynamic_est_lst_does_not_duplicate_tasks`
- `dynamic_est_lst_respects_block_dependencies`
- `dynamic_start_mid_forward_front_respects_mid_cursor`
- `dynamic_start_mid_forward_mid_continues_to_horizon`
- `dynamic_start_mid_forward_no_overlap`
- `dynamic_start_mid_forward_does_not_duplicate_tasks`
