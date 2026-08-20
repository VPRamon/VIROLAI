# Scheduler algorithms

This directory documents the scheduling algorithms available in VIROLAI.

The shared cursor engine implements beam search for EST, LST, and multi-cursor scheduling. EST and LST configure that engine with one cursor. Multi-cursor layouts use several cursors over the same schedule. HAP is a separate planner family.

## Algorithms

| Algorithm | Model | Implementation | Typical use |
| --- | --- | --- | --- |
| EST | One forward cursor | `EstScheduler`, then the cursor engine | Earliest-feasible beam-search baseline |
| LST | One backward cursor | `LstScheduler`, then the cursor engine | Latest-feasible beam-search baseline |
| Multi-cursor | Several coordinated cursors | `MultiCursorScheduler`, then the cursor engine | Fixed or dynamic horizon partitioning |
| HAP | Adaptive/metaheuristic planner | HAP planner pipeline | Stochastic and population-based search |

## Availability

| Surface | EST | LST | Multi-cursor | HAP |
| --- | --- | --- | --- | --- |
| `schedulers` binary | yes | yes | no | yes |
| `virolai run` | yes | yes | no | yes |
| experiment spec (`lab run` / `virolai sweep`) | yes | yes | yes | yes |

Multi-cursor configurations are currently defined through experiment specifications. The standalone scheduler CLI remains focused on single-run EST, LST, and HAP execution.

## Reference

- [cursor-engine.md](cursor-engine.md): shared beam-search engine, frames, territories, and invariants
- [est.md](est.md): EST configuration and ordering
- [lst.md](lst.md): LST and mirrored-frame semantics
- [multi-cursor.md](multi-cursor.md): fixed and dynamic cursor layouts
- [hap.md](hap.md): HAP planner and parameters
- [figures-of-merit.md](figures-of-merit.md): beam-state scoring functions
- [sweep-configuration.md](sweep-configuration.md): experiment specifications and DB-only sweeps

## Experiment workflow

```bash
virolai sweep --spec <FILE> --run-db .lab/runs.sqlite
lab registry list --run-db .lab/runs.sqlite
lab registry export --out-dir out/results --run-db .lab/runs.sqlite
```

Sweeps create registry rows. Schedule files are materialized only by `lab registry export`.
