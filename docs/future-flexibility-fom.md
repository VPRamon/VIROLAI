# Future-Flexibility Figure of Merit

This document explains the EST `future_flexibility` figure of merit (FOM) as
implemented in the scheduler.

## Purpose

`future_flexibility` ranks EST beam-search states by **how much future
schedulability they preserve**, not just by the soft-constraint quality already
captured by the partial schedule.

In practice, this FOM tries to answer:

- How many tasks are still realistically recoverable from the current cursor?
- How crowded is the remaining time horizon if those tasks are all left alive?
- How much slack do those remaining tasks still have?
- If the above signals are tied, which state already captured better
  soft-constraint value?

That makes it a **feasibility-first** scoring rule. It is designed for
oversubscribed scheduling, where a locally attractive choice can easily destroy
future options.

## Where It Is Used

EST expands child schedules during beam search and scores each child before the
global prune step. The highest-scoring children survive into the next round.

So `future_flexibility` does not directly choose the next task by itself.
Instead, it decides which partial schedules are worth keeping alive after each
expansion.

## Score Formula

The implementation uses:

```text
score = 10 * recoverable_count
        + density_term
        + 0.1 * reserve_term
        + 0.01 * soft_term
```

The weights are intentionally lexicographic in effect:

- `recoverable_count` is the dominant signal.
- `density_term` breaks ties between states with the same recoverable count.
- `reserve_term` is a weaker tie-breaker.
- `soft_term` is only a final tie-breaker.

This means the FOM strongly prefers "keep more tasks alive" over "improve the
quality of what is already placed".

## Term 1: Recoverable Count

`recoverable_count` is:

- number of already placed tasks
- plus number of currently unplaced tasks that still have at least one full
  feasible placement window from the current effective cursor

For an unplaced task to count as recoverable:

1. all its predecessors in the same scheduling block must already be scheduled
2. the usable time from its effective cursor to the horizon end must still
   contain at least one full task-duration unit

If a predecessor is still unscheduled, the task is treated as blocked and does
not contribute to the score.

If predecessors are scheduled but end after the current EST cursor, the task's
effective cursor becomes:

```text
max(current_cursor, latest_predecessor_end)
```

That reduces the task's remaining flexibility and can make it unrecoverable.

## Term 2: Density Term

The density term measures how crowded the future horizon is if the recoverable
tasks are all left available.

For each recoverable unplaced task:

1. compute its residual flexibility `F`
2. assign a uniform load contribution of `1 / F`
3. spread that load across every usable overlap window from the effective cursor

The implementation then sweeps all interval endpoints and integrates the area
where the accumulated load exceeds `1.0`:

```text
overload_area = integral(max(load(t) - 1, 0)) dt
density_term  = 1 / (1 + overload_area / remaining_horizon)
```

Interpretation:

- `density_term` is near `1.0` when the future is roomy.
- `density_term` gets smaller when many tasks compete for the same remaining
  time.
- It penalizes temporal crowding, not just task count.

This is what makes the FOM prefer schedules that leave a less congested future.

## Term 3: Reserve Term

The reserve term measures the average residual slack of recoverable unplaced
tasks:

```text
reserve_term = mean(1 - 1 / flexibility)
```

Properties:

- a task with `flexibility = 1` contributes `0`
- a task with large flexibility contributes a value approaching `1`
- the term is `0` when there are no recoverable unplaced tasks

This rewards states where surviving tasks do not merely "still fit", but still
fit with margin.

## Term 4: Soft Term

The final term preserves a tiny amount of already-captured quality:

```text
soft_term = clamp(sum(soft scores of placed tasks) / total_task_count, 0, 1)
```

This is intentionally weak:

- it prevents the FOM from completely ignoring quality
- it does not override feasibility-first behaviour

In other words, soft score matters here mostly when two states are already very
similar in future schedulability.

## Residual Flexibility

The notion of "flexibility" is central to this FOM.

For a task with duration `d`, each usable overlap window contributes:

```text
overlap_duration / d
```

but only if `overlap_duration >= d`.

Examples:

- one exact-fit window contributes `1.0`
- two exact-fit windows contribute `2.0`
- one window of length `3d` contributes `3.0`
- a leftover overlap shorter than `d` contributes `0`

So flexibility is best understood as:

"How many task-length units of feasible time remain for this task?"

This is not a raw count of windows. It combines both **number of options** and
**width of options**.

## Interaction With EST Parameters

`future_flexibility` is most useful when EST is allowed to keep alternatives
alive:

- `--est-k` controls how many partial schedules survive each round
- `--est-b` controls how many local next-task choices are explored per state
- `--est-e` still affects the EST candidate ordering before the FOM is applied

Important detail:

- `--est-e` belongs to the candidate-ordering logic
- `--est-fom future_flexibility` belongs to the beam-state scoring logic

These mechanisms are complementary, not redundant.

With `k = 1` and `b = 1`, EST degenerates to greedy single-path search, so the
FOM still matters, but there is much less room for it to influence search
diversity. With larger `k` and `b`, the effect becomes much stronger.

## Comparison With `soft_constraint`

### What `soft_constraint` does

`soft_constraint` simply sums the soft-constraint score of already placed tasks.

So it asks:

"How good is the partial schedule I have already built?"

### What `future_flexibility` does

`future_flexibility` instead asks:

"How much future scheduling potential did this partial schedule preserve?"

### Practical difference

`soft_constraint` tends to favour:

- immediately high-value placements
- schedules that look good early
- exploitation of already visible reward

`future_flexibility` tends to favour:

- states that keep more tasks schedulable
- schedules that avoid painting themselves into a corner
- exploration of alternatives that preserve optionality

### When `future_flexibility` wins

It is usually the better heuristic when:

- the problem is strongly oversubscribed
- many tasks share overlapping windows
- early greedy choices can block large parts of the future
- dependencies reduce downstream freedom

### When `soft_constraint` may win

It can be preferable when:

- soft-constraint value is the main objective
- the schedule is lightly constrained and feasibility is easy anyway
- preserving future options matters less than grabbing immediate reward
- runtime needs to stay as low as possible

## Complexity

Per FOM evaluation, the dominant work is:

1. scan unplaced tasks
2. inspect their feasible windows after the effective cursor
3. build density sweep events for recoverable tasks
4. sort those events
5. compute the placed-task soft score

Using:

- `U` = number of unplaced tasks
- `P` = number of placed tasks
- `D` = predecessor edges touched while computing effective cursors
- `W` = usable window overlaps inspected
- `E` = density sweep events, with `E <= 2W`

the cost is approximately:

```text
O(U + D + W + E log E + P)
```

The sorting step in the density sweep is usually the dominant extra cost beyond
plain EST bookkeeping.

By comparison, `soft_constraint` is much cheaper:

```text
O(P)
```

This difference matters because EST scores every child state during beam
expansion. If `k` and `b` are increased, the cost of `future_flexibility`
scales up accordingly.

## Strengths

- Strongly protects future schedulability.
- Better aligned with oversubscribed search than immediate-quality scoring.
- Handles dependency-aware recoverability explicitly.
- Penalizes temporal crowding, not just lack of windows.
- Keeps soft score as a deterministic tie-breaker.

## Weaknesses

- More expensive than `soft_constraint`.
- Can prefer "keep options open" over "take the best visible reward now".
- The heavy weight on `recoverable_count` may under-emphasize quality
  differences between states.
- Because soft score is intentionally tiny in the final formula, high-quality
  early placements may be sacrificed if they reduce future feasibility.

## What To Expect In Practice

When switching from `soft_constraint` to `future_flexibility`, expect:

- more conservative search behaviour
- more emphasis on preserving schedulability of the remaining pool
- better robustness on hard, crowded, oversubscribed instances
- less eagerness to lock in a locally attractive task
- higher runtime, especially with larger beam width and branching factor

If you care primarily about final task count or about avoiding catastrophic
early choices, this FOM is usually a good fit.

If you care primarily about maximizing soft reward on easy instances, the
simpler `soft_constraint` FOM may remain more appropriate.

## Recommended CLI Usage

Use the long EST flags:

```bash
cargo run --release --bin phd -- run data/isdc_n.json \
  --algorithm est \
  --est-fom future_flexibility \
  --est-e 5 \
  --est-k 8 \
  --est-b 4 \
  --output out/est-future-flex.json
```

Short forms such as `-e`, `-k`, and `-b` are not currently supported by the
`scheduler` CLI or by `phd run`, which forwards arguments to `scheduler`
unchanged.
