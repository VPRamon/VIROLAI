# Scheduler Algorithms

This directory is the reference guide for the scheduler architecture used in this repository.

The key design point is simple: **the shared cursor engine is the formal beam-search engine** for `est`, `lst`, and `multi_cursor`. EST and LST are public wrappers that preconfigure the engine as a single cursor; multi-cursor layouts use the same engine with several cursors sharing one schedule. HAP is a separate planner family.

## Which algorithm should I use?

| Algorithm | Mental model | Implementation path | Typical use |
|---|---|---|---|
| **EST** | Earliest feasible scheduling with one forward cursor | `EstScheduler` → `MultiCursorConfig::single_forward` → cursor engine | Single-run baseline, beam search from the horizon start |
| **LST** | Latest feasible scheduling with one backward cursor | `LstScheduler` → `MultiCursorConfig::single_backward` → cursor engine | Single-run latest-start baseline |
| **Multi-cursor** | Several cursors share one global schedule | `MultiCursorScheduler` → cursor engine | Fixed or dynamic horizon partitioning experiments |
| **HAP** | Hybrid adaptive/metaheuristic planner | `HapScheduler` planner pipeline | Stochastic or population-based planning |

## Where is each algorithm available?

| Surface | EST | LST | Multi-cursor | HAP |
|---|---|---:|---:|---:|
| Raw `schedulers` binary | ✅ | ✅ | ❌ | ✅ |
| `phd run` | ✅ | ✅ | ❌ | ✅ |
| `lab run` / `phd sweep` spec | ✅ | ✅ | ✅ | ✅ |

Multi-cursor is intentionally configured through experiment specs today. The standalone scheduler CLI remains focused on single-run `est`, `lst`, and `hap`.

## Reading guide

- [cursor-engine.md](cursor-engine.md) — shared beam-search engine, frames, territories, invariants
- [est.md](est.md) — EST wrapper, ordering, parameters, examples
- [lst.md](lst.md) — LST wrapper, mirrored frame semantics, examples
- [multi-cursor.md](multi-cursor.md) — fixed and dynamic layouts, live boundaries
- [hap.md](hap.md) — HAP planner family and parameters
- [figures-of-merit.md](figures-of-merit.md) — `soft_constraint`, `future_flexibility`, scoring semantics
- [sweep-configuration.md](sweep-configuration.md) — DB-only sweep workflow and experiment-spec reference

## Workflow reminder

The primary experiment workflow is **DB-only**:

1. `phd sweep --spec ... --run-db ...` or `lab run --spec ... --run-db ...`
2. Inspect rows with `lab registry list` / `inspect` / `best`
3. Materialize schedule JSON later with `lab registry export`

Sweeps create **registry rows**, not schedule files. Export is the step that writes self-contained schedule artifacts to disk.
