# Environment algorithm analysis plots (EST)

This guide explains the **EST-specific charts** shown under an environment's
**Algorithm analysis** page.

Open it from an environment card with **Algorithm analysis**.

Route family:

- `/environments/:envId/algorithm`
- `/environments/:envId/algorithm/est/:tabId`

This page only works for schedules that were uploaded with an EST trace file such
as `*.est_trace.jsonl`.

> For experiment generation and trace import workflow, see
> [`../est-intelligence-guide.md`](../est-intelligence-guide.md).
>
> For an interpretation of the `out/run-20260427T143841-253798026Z/`
> sweep where larger branching factors reduced scheduling rate and priority
> capture, see
> [`est-sweep-results-interpretation.md`](est-sweep-results-interpretation.md).

## How to think about this page

The compare page tells you **which run looks better**.

The EST algorithm analysis page tells you:

- how the **EST parameters** changed the outcome
- which runs lie on the best **trade-off frontier**
- how the EST search behaved internally round by round

For EST, the main knobs are usually:

- **e** — endangered threshold
- **k** — beam width
- **b** — branching factor

## Tabs without plots

Two tabs are mostly summary views rather than charts:

- **Overview** — headline cards plus a run inventory table
- **Statistics** — per-metric summary table and correlations

The rest of this document focuses on the tabs that contain plots.

## Sweep tab

The Sweep tab is the most direct answer to: *How do e, k, and b affect results?*

### Scheduling rate / Scheduled count / Priority capture vs e, k, or b

This is a **line chart** whose X axis is the currently selected parameter.

- **X axis:** one EST knob (`e`, `k`, or `b`)
- **Y axis:** the selected outcome metric
- **Each line:** one fixed combination of the other two knobs

Example: if the X axis is `e`, then each line groups runs with the same `k` and `b`.

### How to read it

- An **upward slope** means increasing that parameter helps for that line's fixed context.
- A **flat line** means that parameter has little effect there.
- **Crossing lines** mean the best setting depends on the values of the other knobs.

### What it is good for

Use this chart to find:

- which parameter has the strongest effect
- where returns start to flatten
- whether a setting is robust or only good in one narrow region

### 3D parameter space

This is a **3D scatter plot** of all loaded EST runs.

- **X axis:** `e`
- **Y axis:** `k`
- **Z axis:** `b`
- **Color:** the selected metric
- **Each point label:** the schedule name

### How to read it

Look for clusters of bright or dark points:

- bright/high-value regions indicate promising parameter combinations
- isolated good points may indicate a narrow sweet spot
- smooth gradients suggest a stable parameter response

This plot is best for seeing the **shape of the search space** rather than exact values.

## Sensitivity tab

The Sensitivity tab generalizes beyond fixed EST labels and reads the numeric
dimensions present in each run's `algorithm_config`.

For EST, these dimensions usually map to `endangered_threshold`, `k_beams`, and
`branching_factor`.

### Configuration cube

This is a **3D scatter plot** using the first three numeric configuration dimensions.

- **Axes:** the first three numeric config fields found in the run set
- **Color:** the selected metric
- **Points:** schedules/runs

### How to read it

Use it like a response surface:

- clusters of strong colors show good regions
- a diagonal pattern suggests the metric depends on combinations, not a single knob
- scattered colors suggest noisy or weak parameter influence

### Metric vs first numeric dimension

This is a **2D scatter plot** of the chosen metric against the first numeric dimension.

- **X axis:** first numeric config dimension
- **Y axis:** selected metric
- **Point labels:** schedule names

### How to read it

This is the clearest plot for spotting a simple one-parameter trend:

- upward cloud → larger values tend to help
- downward cloud → larger values tend to hurt
- wide vertical spread at the same X value → other parameters matter a lot too

### Parallel coordinates

This chart draws **one polyline per run** across all numeric config dimensions and the selected metric.

- **Each vertical axis:** one parameter or the chosen metric
- **Each line:** one run
- **Color:** the selected metric

### How to read it

Parallel coordinates are useful for pattern hunting:

- lines that end high on the metric axis show good runs
- if those lines also cluster high or low on one parameter axis, that parameter likely matters
- if strong runs split into different paths, there may be several good operating regions

This plot is good for spotting **interactions** that are hard to see in a simple scatter plot.

## Pareto tab

### Pareto front

This is a **3D trade-off plot** across three objectives:

- **X axis:** scheduling rate (%), higher is better
- **Y axis:** priority capture (%), higher is better
- **Z axis:** fragmentation, lower is better

Points are split into:

- **Pareto front** — runs that are not strictly worse than another run on all objectives
- **Dominated** — runs that another run beats overall

### How to read it

A run is on the Pareto front when improving one objective would require giving up
something on another objective.

This means the Pareto front is the set of **serious candidate solutions**.

### What this plot is for

Use it when there is no single "best" run and you need to choose based on
operational preference:

- maximize throughput
- maximize high-priority capture
- minimize fragmentation

## Internals tab

The Internals tab explains what EST did **during the search**, not just what the
final schedule looks like.

### Score trajectory (best / median / worst per round)

This is a multi-line chart built from the EST trace.

- **X axis:** round
- **Y axis:** EST internal score
- For each run:
  - **solid line:** best score in the beam
  - **dotted line:** median score
  - **dashed line:** worst score

### How to read it

- If the **best** line rises quickly then plateaus, the search finds good candidates early.
- A large gap between **best** and **worst** means the beam is diverse.
- Lines collapsing together suggest the beam is converging.

### Important note

This score is the EST trace's **internal fitness/figure-of-merit**, not the same
thing as the compare page's composite score.

### Beam-score distribution per round

This is a **heatmap** for the first selected run that has trace data.

- **X axis:** round
- **Y axis:** beam rank, from best to worst
- **Color:** score

### How to read it

- A bright top row with darker lower rows means only a few beam candidates are strong.
- A broad bright band means many candidates remain competitive.
- Sudden color changes between rounds may indicate a major branching or pruning event.

This plot is the quickest way to see whether EST is exploring broadly or collapsing early.

### Wall time per round

This line chart shows the runtime cost of each EST round.

- **X axis:** round
- **Y axis:** wall time in milliseconds
- **Each line:** one run

### How to read it

- stable low lines mean predictable round cost
- spikes suggest especially expensive expansion or evaluation steps
- later-round growth can indicate increasing search cost as the beam evolves

Use this chart together with the score trajectory: a run that gains little score
while taking much more time may not be worth the extra cost.
