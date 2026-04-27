# Environment analysis manual

This folder explains the **plots shown on environment pages** in the webapp, so users can tell what each chart means and how to read it.

## Where these pages live

From **Workspace** (`/workspace`), each environment card can open:

- **Open compare** → `/environments/:envId/compare`
- **Algorithm analysis** → `/environments/:envId/algorithm`

## Before you open them

- **Compare** needs at least **2 schedules** in the environment.
- **Algorithm analysis** needs schedules uploaded with a matching
  `*.{algorithm}_trace.jsonl` file.

## Guides in this folder

- [compare.md](./compare.md) — all charts on the environment **Compare** page
- [algorithm-analysis-est.md](./algorithm-analysis-est.md) — all EST charts on the
  environment **Algorithm analysis** page

## Scope

This manual focuses on **plots and charts**. Some environment pages also contain
tables and summary cards. Those are mentioned where needed, but the main goal of
this folder is to explain the visual analysis panels.
