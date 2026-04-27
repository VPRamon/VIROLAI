# EST sweep report interpretation

This document explains the EST parameter sweep reports produced under
`out/run-20260427T143841-253798026Z/`, with special attention to the surprising
case where higher exploration settings produce lower scheduling rate and lower
priority capture.

The short version is: the observed trend is compatible with the current EST
beam-search design. It is not, by itself, evidence of a broken queue or an
off-by-one implementation bug. Larger exploration settings currently give the
search more ways to choose locally high-scoring partial schedules, but the
figure of merit used for pruning does not estimate the value of tasks that will
be lost later.

## Source reports

The run directory contains:

| File | Role |
|---|---|
| `manifest.json` | Metadata for the sweep: input file, output directory, baseline slug, and all run configurations. |
| `comparison.csv` | Per-run summary metrics, including scheduled task count and cumulative priority score. |
| `schedules/*.json` | Final schedule produced by each EST configuration. |
| `schedules/*.est_trace.jsonl` | Per-round EST trace for each configuration, used by the algorithm-analysis pages. |

The sweep uses these EST parameters:

| Slug field | EST parameter | Meaning |
|---|---|---|
| `e...` | `endangered_threshold` | Promotes tasks with low flexibility before tasks that would obstruct them. |
| `k...` | `k_beams` | Number of partial schedule states kept after each beam-search round. |
| `b...` | `branching_factor` | Number of schedulable candidates explored from each beam per round. |

The baseline is `e1-k1-b1`, which is the classic greedy EST configuration:
one live beam and one branch per round.

## What the comparison report shows

The inspected comparison file has 216 runs. The headline values are:

| Metric | Run | Value |
|---|---:|---:|
| Baseline scheduled tasks | `e1-k1-b1` | 2221 |
| Baseline cumulative priority | `e1-k1-b1` | 27079.250 |
| Best scheduled task count | `e8-k1-b1` | 2231 |
| Best cumulative priority | `e8-k4-b1` | 27313.100 |
| Worst scheduled task count | `e1-k32-b32` | 2134 |
| Worst cumulative priority | `e1-k32-b32` | 26246.775 |

### Effect of `e`

For the greedy case (`k=1`, `b=1`), increasing `e` improves the result until
about `e=8`, after which the result plateaus:

| `e` | Scheduled tasks | Cumulative priority |
|---:|---:|---:|
| 1 | 2221 | 27079.250 |
| 2 | 2228 | 27276.975 |
| 4 | 2230 | 27301.225 |
| 8 | 2231 | 27313.100 |
| 16 | 2231 | 27313.100 |
| 32 | 2231 | 27313.100 |

This is a plausible outcome: protecting more endangered tasks can prevent early
choices from blocking tasks that have fewer feasible opportunities. Once the
important endangered cases are already protected, increasing the threshold
further does not change much.

### Effect of `b`

Averaged across all `e` and `k` values, increasing `b` lowers both scheduled
task count and cumulative priority:

| `b` | Mean scheduled tasks | Mean cumulative priority |
|---:|---:|---:|
| 1 | 2228.7 | 27266.1 |
| 2 | 2211.4 | 27067.4 |
| 4 | 2191.9 | 26844.2 |
| 8 | 2185.1 | 26780.5 |
| 16 | 2165.8 | 26596.4 |
| 32 | 2149.1 | 26485.4 |

This is the surprising part of the report: more branching does not behave like
"more intelligence" under the current pruning score.

Examples from fixed `e,k` slices:

| Slice | `b=1` | `b=32` |
|---|---:|---:|
| `e1-k1` | 2221 tasks / 27079.3 priority | 2143 tasks / 26355.9 priority |
| `e1-k32` | 2221 tasks / 27079.3 priority | 2134 tasks / 26246.8 priority |
| `e8-k1` | 2231 tasks / 27313.1 priority | 2153 tasks / 26532.4 priority |
| `e32-k32` | 2231 tasks / 27313.1 priority | 2154 tasks / 26547.4 priority |

## Why higher exploration can make the reports worse

The EST search expands and prunes partial schedules round by round:

1. Each live beam refreshes the candidate queue from the beam cursor to the end
   of the horizon.
2. The first `b` schedulable candidates in EST order are each tried as a child
   branch.
3. All children from all live beams are scored.
4. The children are sorted by score.
5. Only the top `k` children survive to the next round.

In the implementation, this pruning happens in `src/scheduler/est/beam.rs`.
The key behavior is:

```rust
next_beams.sort_by(|a, b| b.score.total_cmp(&a.score));
next_beams.truncate(k);
```

The score used here comes from the configured EST figure of merit. For the
current sweep, `manifest.json` records `fom: "soft_constraint"`, which maps to
`SoftConstraintFom`.

That FOM is:

```rust
schedule
    .placements()
    .map(|placement| /* soft-constraint score of placed task */)
    .sum()
```

In other words, the pruning score is the cumulative soft-constraint value of
tasks that have already been placed. It does not include:

- the number of tasks still schedulable after this choice
- the priority of tasks that may become impossible later
- remaining usable time
- fragmentation
- estimated future yield
- an explicit penalty for placing a long or awkward task now

So larger `b` does not simply mean "try more and keep the globally best path".
It means "try more local alternatives, then keep the partial schedules that
currently have the highest placed priority".

That can be harmful. A branch that places a high-priority task early may score
well immediately, but it may also consume a large or strategically important
time interval. If that later blocks several medium-priority tasks, the final
schedule can have both:

- fewer scheduled tasks
- lower cumulative priority

The report is therefore consistent with a myopic pruning objective.

## Why this is not automatically a bug

It is natural to expect that higher exploration should dominate greedy EST. That
would be true only if the greedy trajectory were guaranteed to remain available
until final selection, or if the pruning score were a reliable predictor of
final quality.

The current algorithm does not guarantee either condition.

### The greedy path can be pruned

For any beam, branch index `0` is the EST-ordered greedy candidate. However,
after all children are scored, pruning is global across the round. If `k` other
children have higher current FOM scores, the greedy child is discarded. Once a
path is discarded, it cannot re-enter final selection.

This means a larger `b` can actually increase the number of tempting alternatives
that outscore the greedy branch locally.

### `k` is not elitism

Increasing `k` keeps more beams alive, but it still keeps them according to the
same myopic FOM. A larger beam is not the same thing as preserving a known-good
reference path. Without an explicit elitism rule, the `e1-k1-b1` trajectory is
not protected.

### Final selection uses the same cached score

When all live beams terminate, the final schedule is selected by the same cached
score. This is internally consistent, but it means the final choice is limited
to paths that survived earlier FOM-based pruning.

## How to read the priority numbers carefully

The `comparison.csv` file reports:

- `scheduled_task_count`
- `fitness_priority_sum`
- scheduled-priority quantiles (`p25`, `p50`, `p75`, `p90`)

In this sweep, the scheduled-priority quantiles are identical across the printed
rows:

| Quantile | Value |
|---|---:|
| p25 | 8.5 |
| p50 | 11.875 |
| p75 | 12.125 |
| p90 | 19.2 |

This means the broad distribution of scheduled task priorities is not changing
much. The main difference visible in `comparison.csv` is the number of tasks and
the cumulative priority sum, not a dramatic shift in the median scheduled task.

If the webapp shows "priority capture", check whether it is using cumulative
priority, mean priority, normalized captured priority, or a composite score.
Those metrics answer different questions:

| Metric | Interpretation |
|---|---|
| Cumulative priority | Total priority value scheduled; usually falls when fewer tasks are scheduled. |
| Mean scheduled priority | Average value of scheduled tasks; can rise even if total capture falls. |
| Priority capture percentage | Fraction of all available priority captured; should be compared with scheduled count. |
| Priority distribution | Describes scheduled tasks only; does not show high-priority tasks left unscheduled. |

The report can therefore show a lower total priority even if individual selected
tasks are not obviously lower priority.

## Interpretation of the current sweep

The best explanation of the current reports is:

1. Raising `e` improves greedy EST because endangered-task promotion avoids some
   avoidable blocking.
2. Raising `b` exposes more candidate branches.
3. The current FOM ranks these branches by already-captured soft-constraint
   score.
4. The search then prefers partial schedules that look strong immediately.
5. Some of those schedules damage future schedulability enough that the final
   schedule is worse.

This explains the simultaneous drop in scheduling rate and cumulative priority.
It also explains why the drop is systematic with `b`: larger branching gives the
myopic FOM more chances to select a locally attractive but globally poor branch.

## What would be suspicious

The current report pattern is plausible, but these signs would be stronger
evidence of an implementation bug:

- `b=1` changing results when only `k` changes and the initial path should be
  identical.
- The same run slug producing different results across repeated deterministic
  runs.
- Trace files showing zero children when schedulable candidates exist.
- A branch index greater than zero removing the wrong candidate from a cloned
  queue.
- A final selected schedule with a lower FOM than another terminal schedule that
  survived to final selection.
- Schedules violating task dependencies or visibility constraints.

The inspected beam code does not show an obvious issue in these areas: child
states are cloned before popping branch candidates, pruning is explicit and
score-based, and terminal selection uses the same score ordering.

## Practical conclusion

The reports should be read as evidence that the current EST beam search is
limited by its pruning objective, not necessarily that EST placement itself is
broken.

For research interpretation:

- `e` appears useful in this dataset, with a plateau around `e=8`.
- `b>1` should not be described as universally better exploration for the
  current FOM.
- `k` and `b` are only beneficial if the pruning score preserves paths that are
  good for final schedule quality, not just good for immediate placed priority.
- The greedy `b=1` runs are valid baselines and, in this sweep, are often better
  than wider branching runs.

No implementation change is made by this document.

