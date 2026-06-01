# Figures of Merit

Figures of merit (FOMs) rank candidate beam states. They **do not** determine schedule validity.

The cursor engine enforces validity first:

- no duplicate task
- no overlap
- active-region containment
- block dependencies

Only valid children are scored.

## User-facing FOMs

Current sweep/CLI-facing `FomKind` values are:

- `soft_constraint`
- `future_flexibility`

The Rust library also exposes helpers such as `CompositeFom`, but the current CLI/spec surface selects between the two names above.

## `soft_constraint`

`SoftConstraintFom` scores partial schedules from already-captured soft-constraint quality. It is the default FOM and the most conservative choice.

Best fit:

- EST or LST baselines
- fixed or dynamic multi-cursor layouts when you want simple beam ranking

## `future_flexibility`

`FutureFlexibilityFom` is feasibility-first. It tries to preserve the ability to schedule remaining tasks.

### Fallback behavior

When no active-region metadata is present in `FomContext`, it falls back to a single synthetic residual region:

`[ctx.cursor, ctx.horizon.end]`

That preserves legacy single-frontier behavior for unit tests and older call sites.

### Cursor-engine behavior

When the shared cursor engine populates `ctx.active_periods`:

1. every non-empty active region is considered
2. for each unplaced task, residual flexibility is evaluated separately in each region
3. the task is assigned to the region where it keeps the **maximum** residual flexibility
4. the task is recoverable if that best flexibility is `>= 1.0`
5. density is normalized by the **total active-region duration**

This is multi-cursor-aware behavior: the FOM does not “sum flexibility across cursors”. It chooses the best surviving region for each task.

### Dependency behavior

- if a predecessor is unscheduled, the task is blocked
- if predecessors are scheduled, the effective cursor is clipped by the latest predecessor end

So dependency constraints reduce residual flexibility before the task is counted as recoverable.

### Score components

`FutureFlexibilityFom` combines:

- recoverable task count
- density term
- reserve/slack term
- soft-constraint term

The exact weights live in the implementation and are designed to keep recoverability lexicographically dominant.

## Choosing a FOM

| FOM | Best use |
|---|---|
| `soft_constraint` | Stable default, prioritize already-captured quality |
| `future_flexibility` | Preserve future schedulability, especially when beam branching matters |

## Relevant code

- `schedulers/src/scheduler/fom/mod.rs`
- `schedulers/src/scheduler/fom/soft_constraint.rs`
- `schedulers/src/scheduler/fom/future_flexibility.rs`
