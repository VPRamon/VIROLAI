# Sweep Configuration

Sweeps are **DB-only**. They write results into the SQLite registry and materialize schedule JSON later with `lab registry export`.

## Workflow

1. write an experiment spec
2. run `phd sweep --spec ... --run-db ...` or `lab run --spec ... --run-db ...`
3. inspect registry rows
4. export selected schedules when needed

## Top-level fields

```json
{
  "name": "paper-sweep",
  "max_parallel": 8,
  "output_dir": "out/legacy-only",
  "datasets": [],
  "algorithms": []
}
```

| Field | Meaning |
|---|---|
| `name` | Human-readable experiment name |
| `max_parallel` | Optional worker cap |
| `output_dir` | Legacy field; accepted but ignored by the DB-only runner |
| `datasets` | Dataset entries |
| `algorithms` | Per-algorithm sweep blocks |

## Dataset entries

```json
{
  "id": "isdc_n",
  "path": "data/isdc_n.json",
  "label": "SDC North",
  "horizon_override": {
    "start_mjd": 61771.0,
    "end_mjd": 61781.0
  }
}
```

## Corrected minimal example

```json
{
  "name": "my-experiment",
  "datasets": [
    {
      "id": "isdc_n",
      "path": "data/isdc_n.json",
      "label": "SDC North"
    }
  ],
  "algorithms": [
    {
      "kind": "est",
      "axes": {
        "endangered_thresholds": [1, 2],
        "k_beams": [1, 4],
        "branching_factors": [1, 2]
      }
    }
  ]
}
```

This produces **1 dataset × 2 × 2 × 2 EST cells = 8 registry rows** when all runs succeed.

## Algorithm blocks

### EST

```json
{
  "kind": "est",
  "axes": {
    "endangered_thresholds": [0, 1, 2],
    "k_beams": [1, 4],
    "branching_factors": [1, 2],
    "foms": ["soft_constraint", "future_flexibility"]
  }
}
```

### LST

LST uses the same axes as EST:

```json
{
  "kind": "lst",
  "axes": {
    "endangered_thresholds": [0, 1, 2],
    "k_beams": [1, 4],
    "branching_factors": [1, 2],
    "foms": ["soft_constraint", "future_flexibility"]
  }
}
```

### Multi-cursor

```json
{
  "kind": "multi_cursor",
  "axes": {
    "layouts": ["est_lst_split", "dynamic_est_lst_meet"],
    "endangered_thresholds": [1],
    "k_beams": [4],
    "branching_factors": [2],
    "foms": ["soft_constraint", "future_flexibility"]
  }
}
```

### HAP

```json
{
  "kind": "hap",
  "axes": {
    "iota_max_values": [64, 128],
    "rho_values": [3, 5],
    "population_sizes": [4, 8],
    "survivor_modes": ["elitist_top_k"],
    "survivor_caps": [4],
    "seeds": [0, 1, 2]
  }
}
```

## What sweeps produce

Sweeps create:

- one **run row** per executed configuration
- one **deduplicated schedule row** per semantically unique schedule body

They do **not** directly create schedule files. Files appear only after:

```bash
lab registry export --out-dir out/my-sweep --run-db .lab/runs.sqlite
```

## Related docs

- [README.md](README.md)
- [est.md](est.md)
- [lst.md](lst.md)
- [multi-cursor.md](multi-cursor.md)
- [hap.md](hap.md)
