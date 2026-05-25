# EST Algorithm Logic

This module implements the **Earliest-Start-Time** scheduler as a beam-search
variant. That means the algorithm does not follow only one greedy path: at each
step it may keep multiple partial schedules alive and explore multiple next
choices.

Core files:

- `algorithm.rs` prepares and starts the run.
- `candidate.rs` computes the per-task scheduling state.
- `ordering.rs` defines the candidate ordering rules.
- `queue.rs` keeps the candidate queue up to date.
- `beam.rs` runs the actual beam-search loop.
- `context.rs` enforces in-block dependency checks.

## Main idea

EST always works relative to a **cursor**. The cursor is the time before which
the next placement may no longer start. Initially, the cursor is the start of
the global horizon.

Each live beam state contains:

- the partial schedule built so far
- the current cursor time
- the candidate queue for all not-yet-placed tasks
- the score of that partial schedule

At each iteration the algorithm:

1. refreshes all candidates relative to the cursor
2. finds the schedulable prefix of the ordered queue
3. creates up to `branching_factor` child branches
4. scores each child with the selected FOM
5. keeps only the best `k_beams` children overall

If a branch can no longer place anything, it becomes a terminal state. At the
end, the algorithm returns the highest-scoring schedule among all terminal
states.

## 1. Candidate refresh

In EST, each task is represented as a `Candidate`. Its key fields are:

- `est`: earliest feasible start
- `deadline`: latest feasible start
- `flexibility`: how much usable time remains

Refreshing a candidate works like this:

1. discard windows that are entirely behind the cursor
2. inspect overlaps between remaining windows and the active horizon slice
3. ignore overlaps shorter than the task duration
4. use the first valid overlap start as `est`
5. add every valid overlap to `flexibility`

`flexibility` is not just raw free time. It is normalized by task duration:

`flexibility += usable_overlap_duration / task_duration`

So:

- `flexibility < 1.0` means the task can no longer be fully scheduled from the
  current cursor
- smaller `flexibility` means less scheduling slack

## 2. Endangered rule

The `endangered_threshold` parameter (`e`) protects tasks with little temporal
slack.

A task is **endangered** when:

`flexibility < endangered_threshold`

If `e = 0`, this protection is completely disabled.

Endangered tasks are not simply moved forward by label. Their effect is applied
through `effective_est`:

- for an endangered task, `effective_est = est`
- for a normal task, `effective_est` starts as `est`
- if a normal task would overlap the early start region of an endangered task,
  its `effective_est` is pushed later

The goal is to avoid letting a flexible task block a task that has very few
remaining opportunities.

## 3. Candidate ordering

Candidates are sorted by a total ordering. The priority is:

1. schedulable candidate before impossible candidate
2. smaller `effective_est`
3. at equal `effective_est`, endangered before non-endangered
4. smaller original `est`
5. smaller `flexibility`
6. higher soft-constraint score at the cursor
7. smaller `task_id`

So in this codebase, EST is not just "pick the earliest start." The endangered
rule and tie-breakers can intentionally override the naive earliest-start
choice.

## 4. Beam-search step

When expanding one beam state, the algorithm:

1. refreshes the candidate queue against the new cursor
2. counts how many candidates are currently schedulable
3. takes up to `b` candidates from the ordered schedulable prefix
4. creates one child state per selected candidate

For each child:

- the chosen candidate is removed from the queue
- the task is placed at `[est, est + duration)`
- the cursor moves to the end of that placement
- the partial schedule is rescored with the FOM

If domain validation rejects the placement, that branch is dropped.

## 5. Constraints and domain checks

EST does not re-check every hard constraint here. It assumes the prescheduler
has already produced `possible_periods` that are hard-constraint-feasible.

It does still enforce in-block dependencies through `context.rs`:

- all predecessor tasks must already be scheduled
- each predecessor must end before the new task starts

If that fails, the branch is pruned.

## 6. Scoring and pruning

Beam search does not keep every branch. After each round:

1. all children from all live beams are collected
2. the children are sorted by score descending
3. only the first `k_beams` survive

By default, the score comes from `SoftConstraintFom`, which sums the
soft-constraint value of every already placed task at its scheduled start time.

That means:

- `b` controls how many next choices each state is allowed to explore
- `k` controls how many partial schedules survive after the round

## 7. Effect of the parameters

The module defaults are:

- `e = 1`
- `k = 1`
- `b = 1`

This is effectively the classic greedy EST mode:

- one live state
- one chosen candidate per step
- endangered protection enabled

Increasing `b` makes the algorithm try more local next-step alternatives per
state. Increasing `k` lets it preserve more competing partial schedules across
rounds. Together they allow the scheduler to escape bad early greedy choices.

## 8. One-sentence summary

In this project, EST means:

**pick the earliest feasible tasks relative to the cursor, protect tasks with
little slack, score the resulting partial schedules with a soft-constraint
figure of merit, and keep only the best branches through beam search.**
