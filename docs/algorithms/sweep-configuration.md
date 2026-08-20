# Sweep configuration

VIROLAI sweeps are DB-only. They store run data in the SQLite registry and materialize schedule JSON files later through `lab registry export`.

## Workflow

1. Write an experiment specification.
2. Run `virolai sweep --spec ... --run-db ...` or `lab run --spec ... --run-db ...`.
3. Inspect the registry.
4. Export selected schedules when needed.

## Top-level fields

```json
{
  "name": "paper-sweep",
  "max_parallel": 8,
  "datasets": [],
  "algorithms": []
}
```

| Field | Meaning |
| --- | --- |
| `name` | Human-readable experiment name |
| `max_parallel` | Optional worker limit |
| `output_dir` | Legacy field; accepted but ignored by the DB-only runner |
| `datasets` | Scheduling problem inputs |
| `algorithms` | Algorithm sweep definitions |

## Dataset entries

A dataset entry identifies any scheduling problem compatible with the common schema:

```json
{
  "id": "sample",
  "path": "data/isdc_n.json",
  "label": "Sample dataset",
  "horizon_override": {
    "start_mjd": 61771.0,
    "end_mjd": 61781.0
  }
}
```

The scheduler does not attach domain-specific meaning to the dataset identifier or path.

## Minimal example

```json
{
  "name": "my-experiment",
  "datasets": [
    {
      "id": "sample",
      "path": "data/isdc_n.json",
      "label": "Sample dataset"
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

This example defines eight EST cells when all axis values are combined.

## Algorithm blocks

### EST and LST

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

Use `"kind": "lst"` for the equivalent LST sweep.

### Multi-cursor

```json
{
  "kind": "multi_cursor",
  "axes": {
    "layouts": ["est_lst_split", "four_quarter_forward"],
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

## Outputs

Sweeps create one run row per executed configuration and one deduplicated schedule row per semantically unique schedule body. They do not write schedule files directly.

Export files explicitly:

```bash
lab registry export \
  --out-dir out/my-sweep \
  --run-db .lab/runs.sqlite
```
