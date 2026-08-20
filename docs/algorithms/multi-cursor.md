# Multi-cursor

Multi-cursor scheduling runs several cursors over one shared schedule.

Each cursor has its own identifier, direction, territory, frame, and candidate queue. When one cursor places a task, that task is removed from every other cursor queue.

Production multi-cursor scheduling follows this path:

`MultiCursorScheduler` -> `engine::run_multi_cursor(...)`

The same engine also powers the EST and LST wrappers.

## Fixed layouts

`est_lst_split` uses a forward cursor over the first half of the horizon and a backward cursor over the second half.

`start_mid_forward` uses two forward cursors with a fixed midpoint split.

`four_quarter_forward` uses four forward cursors, one for each quarter of the horizon.

## Dynamic layouts

`dynamic_est_lst_meet` starts one cursor at each horizon boundary. Each cursor's inner boundary follows the live position of the other cursor so the two frontiers cannot cross.

`dynamic_start_mid_forward` starts one forward cursor at the horizon start and another at the midpoint. The first cursor's end boundary follows the second cursor's live position.

## Active regions

At each round, the engine recomputes the active region of every cursor from the layout, live cursor state, direction, and configured gap. A cursor with an empty region contributes no action in that round.

## Configuration

Multi-cursor experiments use:

- `layouts`
- `endangered_thresholds`
- `k_beams`
- `branching_factors`
- `foms`

Example:

```json
{
  "kind": "multi_cursor",
  "axes": {
    "layouts": ["est_lst_split", "four_quarter_forward"],
    "endangered_thresholds": [1],
    "k_beams": [4],
    "branching_factors": [2],
    "foms": ["soft_constraint", "future_flexibility"]
  }
}
```

Multi-cursor scheduling is configured through experiment specs (`lab run` or `virolai sweep`) rather than the standalone scheduler CLI.

## Invariants

- all cursors share one global schedule
- a task is placed at most once across all cursors
- placements do not overlap
- each placement stays inside the acting cursor's active region
- scheduling-block dependencies are enforced across cursors
