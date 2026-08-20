# Future-flexibility figure of merit

`future_flexibility` is a beam-state scoring function for cursor-engine scheduling. It ranks partial schedules by the amount of future schedulability they preserve.

## Score

The implementation uses:

```text
score = 10 * recoverable_count
        + density_term
        + 0.1 * reserve_term
        + 0.01 * soft_term
```

The terms are weighted to make recoverability dominant. Density and reserve break ties between states with similar recoverability, while soft score is a final quality signal.

## Recoverable count

A task contributes to `recoverable_count` when it is already placed or when an unplaced task still has at least one full feasible placement window from its effective cursor.

For dependency-constrained tasks, the effective cursor is:

```text
max(current_cursor, latest_predecessor_end)
```

A task whose predecessors are not yet satisfied does not count as recoverable.

## Density term

For each recoverable unplaced task, the implementation derives a load contribution from residual flexibility and distributes that load across usable windows. It then integrates overload above a total load of `1.0`:

```text
overload_area = integral(max(load(t) - 1, 0)) dt
density_term  = 1 / (1 + overload_area / remaining_horizon)
```

The term decreases when many tasks compete for the same future intervals.

## Reserve term

The reserve term measures average residual slack:

```text
reserve_term = mean(1 - 1 / flexibility)
```

A task with flexibility `1` contributes no reserve. Tasks with more alternatives contribute progressively more.

## Soft term

The final term uses the soft-constraint score already captured by placed tasks:

```text
soft_term = clamp(sum(soft scores of placed tasks) / total_task_count, 0, 1)
```

Its weight is intentionally small so it does not override future feasibility.

## Residual flexibility

For a task of duration `d`, each usable overlap window contributes `overlap_duration / d` when the overlap is at least one full task duration. Shorter fragments contribute zero.

This combines the number of feasible alternatives with the width of those alternatives.

## Interaction with EST parameters

`future_flexibility` is most useful when the beam retains alternatives:

- `--est-k` controls the number of partial schedules retained after each round
- `--est-b` controls the number of local actions explored from each state
- `--est-e` affects candidate ordering before beam-state scoring

Candidate ordering and FOM scoring are separate mechanisms.

## Comparison with `soft_constraint`

`soft_constraint` scores value already captured by the partial schedule. `future_flexibility` scores the scheduling potential that remains.

The latter is generally more useful in oversubscribed or highly constrained problems where locally attractive placements can remove many future options. It is also more expensive because it evaluates remaining windows and density events for each child state.

Approximate evaluation complexity is:

```text
O(U + D + W + E log E + P)
```

where `U` is unplaced tasks, `D` dependency edges examined, `W` usable window overlaps, `E` density events, and `P` placed tasks.

## CLI example

```bash
cargo run --release -p lab --bin virolai -- run \
  data/isdc_n.json \
  --algorithm est \
  --est-fom future_flexibility \
  --est-e 5 \
  --est-k 8 \
  --est-b 4 \
  --output out/est-future-flex.json
```
