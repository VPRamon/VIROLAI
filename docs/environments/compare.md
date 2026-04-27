# Environment compare plots

This page is the main **side-by-side schedule comparison** view for one environment.
Open it from an environment card with **Open compare**.

Route: `/environments/:envId/compare`

## What the page is trying to answer

Use this page when you want to know:

- which schedule is best overall for this environment
- which schedules trade one benefit for another
- where time is being used, wasted, or blocked

The **Verdict** block above the plots is important context:

- it picks a **baseline** schedule, defaulting to the oldest uploaded one
- it identifies the current **best** schedule by composite score
- it shows KPI deltas against the baseline

The plots below help you understand **why** the winner looks better or worse.

## KPI evolution

This is a multi-line chart showing how several normalized KPI components change
from one schedule to the next.

- **X axis:** schedule name, ordered by **upload order**
- **Y axis:** normalized score from **0 to 1**, where **higher is better**
- **Each line:** one KPI component

The lines are:

- **Composite score** — the overall combined score
- **Scheduling rate** — fraction of tasks that ended up scheduled
- **Operable time used** — how much usable telescope time was filled
- **Visibility utilisation** — how well the schedule used periods where targets were visible
- **Priority alignment** — how strongly the schedule favored high-priority work
- **Gap compactness** — how well idle time was kept compact rather than fragmented

### How to read it

- If most lines rise together, later schedules are improving broadly.
- If one line rises while another falls, you are looking at a **trade-off**.
- If the composite score rises but one component falls, the gain elsewhere was large enough to compensate.

### Important caveat

The X axis is **upload order**, not parameter order. A zig-zagging line does not
necessarily mean unstable behavior in the algorithm itself; it may just reflect
the order in which runs were imported.

## Key Metrics

This panel contains **four bar charts**, one per metric. Each bar is one schedule.

### 1. Scheduling Rate

- **Higher is better**
- Shows the percentage of tasks that were scheduled

Use it to see which schedule covers the largest share of the block set.

### 2. Cumulative Priority

- **Higher is better**
- Adds the priority values of all scheduled tasks

This answers: *Did the scheduler pick the most valuable work, not just the most work?*

### 3. Scheduled Hours

- **Higher is usually better**
- Total time assigned to scheduled observations

This is the easiest way to compare how much of the schedule window was actually filled.

### 4. Gap Count

- **Lower is usually better**
- Number of idle gaps in the schedule

A schedule with many short gaps is often harder to operate efficiently than one
with fewer, larger idle regions.

## Scheduled Task Priority Distribution

This is a **box plot** for the priorities of the tasks that were actually scheduled.

- **One box per schedule**
- **Y axis:** priority

### How to read the box plot

- the **middle line** is the median scheduled priority
- the **box** is the middle 50% of scheduled priorities
- the **whiskers** show the broader spread
- the **points** are outliers

### What it tells you

- A **higher median** means the schedule tends to prioritize more important tasks.
- A **wide box** means the schedule mixes very different priority levels.
- A **tight, high box** means the schedule consistently picks high-priority work.

### Important caveat

This plot only uses **scheduled** tasks. It does **not** show which high-priority
tasks were left unscheduled.

## Time-Use Breakdown

This is a **100% stacked horizontal bar chart**. Each bar is one schedule, and the
full width represents the entire schedule window.

### Segments

- **Scheduled** — time that was successfully assigned to observations
- **Feasible but unused** — usable time existed, but nothing was scheduled there
- **Visible - no task fits** — at least one target was visible, but no remaining task could fit
- **No target visible** — no target was visible in that period
- **Non-operable** — time outside the operable window

### How to read it

- More **Scheduled** is usually good.
- Large **Feasible but unused** means the scheduler left usable opportunities on the table.
- Large **Visible - no task fits** often points to duration, ordering, or constraint mismatch.
- Large **No target visible** is mostly a property of the problem, not the scheduler.
- Large **Non-operable** means the overall window itself contains a lot of unusable time.

### What this chart is best for

This is the fastest plot for answering:

- *Why did this schedule leave time idle?*
- *Is the lost time caused by the algorithm or by the environment?*

## What to look at next

If the compare plots show meaningful differences and the schedules came from an
algorithm that emits traces, open **Algorithm analysis** to understand how the
algorithm settings and internal search behavior produced those differences.
