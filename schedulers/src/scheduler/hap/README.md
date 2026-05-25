# HAP and AP Algorithm Logic

This module implements the **Accumulative Planner** family:

- **AP**: the deterministic, greedy single-schedule variant
- **HAP**: the stochastic multi-start, multi-survivor variant

Both are built on the same shared core in `accumulative.rs`. In this codebase,
AP and HAP are not two unrelated schedulers. They are two configurations of one
planner that processes the problem block by block and uses the **CRU**
(Conflict Resolution Unit) to generate candidate schedules for each block.

Core files:

- `accumulative.rs` contains the shared planner loop
- `ap.rs` exposes the AP entry point
- `hap.rs` exposes the HAP entry point
- `configuration.rs` defines the knobs that make AP and HAP differ
- `cru/` generates candidate schedules for one block
- `eval.rs` defines the schedule and block metrics
- `selection.rs` selects which schedules survive to the next block

## Main idea

The planner starts from an input schedule and processes the scheduling problem
**one block at a time**.

For each block, it:

1. chooses one or more source schedules from the current survivor set
2. runs CRU on that block starting from each source schedule
3. optionally keeps the unchanged source schedule as a rejection candidate
4. deduplicates identical candidate schedules
5. applies a survivor-selection rule

The output of one block becomes the input survivor set for the next block.

So the algorithm is **accumulative** in a literal sense: it keeps extending and
filtering partial schedules as it moves through the ordered list of blocks.

## 1. Block ordering

Before planning starts, the blocks are sorted by descending `block_priority`.

`block_priority` is:

- the sum of the soft-constraint scores of the block's tasks
- evaluated at `horizon.start`

If two blocks have the same priority, the tie is broken by smaller block id.

This means AP/HAP do not schedule tasks in arbitrary order. They try to handle
the most valuable blocks first.

## 2. Shared accumulative planner loop

The shared control flow in `accumulative.rs` is:

1. initialize `survivors = [input_schedule]`
2. sort all blocks by descending priority
3. for each block:
4. generate candidate schedules from the current survivors
5. reduce those candidates using the configured survivor selector
6. carry the survivors forward to the next block

Important implementation details:

- `population_size` controls how many source schedules are used per block
- sources are pulled from the current survivor set in round-robin order
- each source schedule is cloned before CRU runs
- identical schedules are removed using a canonical placement fingerprint
- if no candidates survive for a block, the planner keeps the previous
  survivor set behaviorally unchanged

If the problem has no blocks, the planner simply returns the input schedule.

## 3. What CRU does

CRU is the local schedule-repair and insertion heuristic used for one block.

Its job is: starting from a source schedule, try to make the current block
complete by inserting its tasks, possibly displacing already placed tasks that
conflict with them.

There are two nested levels:

1. **Block Scheduling Cycle**
   For every valid completion branch of the block, CRU starts from a fresh
   clone of the source schedule and tries to realize that branch.
2. **Task Scheduling Cycle**
   For each task in that branch, CRU runs a lobby-based repair loop that
   inserts the task, evicts conflicting non-protected tasks, and keeps trying
   to reinsert displaced tasks.

If a branch completes the block successfully, its resulting schedule is kept as
a CRU candidate. If multiple branches produce the same placements, duplicates
are removed.

## 4. Completion branches

CRU does not assume every block means "schedule all tasks."

Instead, it enumerates the block's valid **completion branches** from the
completion expression:

- a block may require all tasks
- or one of several alternatives
- or a more complex completion formula

Each completion branch is tried independently from a fresh clone of the source
schedule, so state from one OR-branch does not leak into another.

Only schedules that satisfy the block completion rule are returned as valid CRU
results.

## 5. Task Scheduling Cycle and the lobby

The inner Task Scheduling Cycle is the most important CRU mechanism.

It starts with a **lobby** containing one initial task id. During the cycle:

1. one task is popped from the lobby
2. the algorithm computes valid placement candidates for that task
3. one candidate placement is chosen
4. conflicting non-protected tasks are evicted from the schedule
5. evicted tasks are pushed back into the lobby
6. the chosen task is inserted into the schedule
7. the process repeats until the lobby is empty or `max_iter` is reached

This is a repair loop, not a simple one-shot insertion.

### Protected tasks

CRU is not allowed to evict every overlap freely.

Protected tasks include:

- tasks that belong to the current task's owning block
- tasks already placed successfully earlier in the current task-scheduling run

If a candidate placement would conflict with a protected task, that candidate is
discarded.

## 6. Candidate placements inside CRU

For one task, CRU builds candidate placements from its feasibility windows.

Per window, it considers two kinds of start times:

1. the start of the window
2. the end of each already placed overlapping task inside that window

For each possible start:

- the task must fit fully before the window ends
- conflicts with protected tasks are forbidden
- non-protected conflicts are allowed but counted as insertion cost

The local insertion cost is currently:

- the number of tasks that would be displaced

Candidates are sorted by:

1. lower cost
2. earlier start time
3. fewer conflicts

## 7. How AP and HAP choose local candidates

The CRU inner selector is configured by `Selector`.

Available modes:

- `Deterministic`
  Always pick the first candidate from the sorted list.
- `Stochastic { rho }`
  Pick uniformly from the `rho` cheapest candidates.
- `Random`
  Pick from all candidates with probability weighted inversely by cost.

Special rule:

- if any zero-cost candidate exists, it always wins regardless of selector

That means HAP's stochasticity only affects the non-zero-cost choice region.

## 8. `s_low` rollback behavior

While the Task Scheduling Cycle runs, the implementation tracks the intermediate
schedule with the lowest observed **lobby cost**.

`lobby_cost` is:

- the sum of the cheapest currently available insertion costs for all tasks
  still waiting in the lobby

At the end of the run, the schedule is rolled back to the best snapshot
observed during the cycle, not necessarily the last temporary schedule state.

This is the code equivalent of the CRU `s_low` / `cost_min` recovery rule.

## 9. Rejection candidate

For each source schedule, the planner can also keep the unchanged source itself
as a candidate for that block.

This is controlled by `include_rejection_candidate`, and it is enabled in the
default AP and HAP presets.

Why it matters:

- if every CRU-produced schedule is worse than the current source
- the planner can reject the block instead of forcing a damaging insertion

So a block is not always accepted just because CRU found some feasible result.

## 10. Schedule evaluation metrics

The planner uses several metrics from `eval.rs`.

### `block_priority`

Used to sort blocks before planning starts.

### `completion_fitness`

The main scalar quality score for greedy and elitist selection:

- for each block, compute `(placed_tasks / total_block_tasks) * block_priority`
- sum that over all blocks

Higher is better.

### `science_time`

The sum of all placement durations. Used as a tie-breaker.

### `scheduling_rate`

`placed_task_count / total_task_count`

### `priority_sum`

The sum of `block_priority` over the blocks that are fully complete in the
current schedule.

## 11. Survivor selection

After CRU has produced the candidate schedules for the current block, the
planner reduces them using `selection.rs`.

### `GreedyOne`

Keep exactly one schedule:

1. higher `completion_fitness`
2. higher `science_time`
3. more placements
4. lexicographically smaller placement fingerprint

This is the AP behavior.

### `ElitistTopK`

Keep the top `k` schedules by the same scalar ordering.

This is a common HAP mode.

### `ParetoFront`

Keep the non-dominated schedules over:

- `scheduling_rate`
- `priority_sum`

If the front is larger than `cap`, it is pruned using NSGA-II-style crowding
distance so that extreme trade-off points are preserved.

## 12. AP vs HAP in this repository

The practical difference is entirely configuration.

### AP

AP uses:

- deterministic CRU selector
- `population_size = 1`
- `GreedyOne` survivor selection
- rejection candidate enabled

So AP keeps a single schedule alive, processes blocks in priority order, and
greedily keeps the best candidate after each block.

### HAP

HAP typically uses:

- stochastic CRU selector
- `population_size > 1`
- `ElitistTopK` or `ParetoFront` survivor selection
- rejection candidate enabled
- seeded RNG for reproducible stochastic runs

So HAP keeps a population of schedules alive and explores multiple local CRU
outcomes for each block before pruning the population again.

## 13. Relationship between AP and HAP

In this implementation:

- AP is a special case of the accumulative planner
- HAP is the same planner with stochastic candidate choice and multi-survivor
  control flow

The tests in `accumulative.rs` make this explicit: AP should match HAP when HAP
is configured with population size 1, deterministic selection, and
`GreedyOne`.

## 14. One-sentence summary

In this project, AP/HAP means:

**sort blocks by value, use CRU to try to complete each block from one or more
current schedules, allow local conflict-driven repair through the lobby, then
keep only the best or most interesting resulting schedules before moving to the
next block.**
