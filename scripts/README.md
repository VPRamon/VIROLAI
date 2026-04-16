# Scripts

## `ctao_adapter`

Rust CLI that converts CTA dataset files in one directory (`*_internalSDC.json`) into one aggregated `scheduling_blocks.json` file compliant with `schemas/scheduling_blocks.schema.json` (also available through `data/scheduling_blocks.schema.json` compatibility shim).

The output is a minimal scheduler-ready PhD payload:

- each item has `id`, `tasks`, and `dependencies`
- `tasks` contains task objects (not CTA configuration payloads)
- each task object includes at least `id`, `requested_duration_sec`, `hard_constraints`, and optional `soft_constraints.priority`
- each CTA scheduling block is currently converted to one task object (`tasks: [{...}]`)

### Run

```bash
cargo run --bin ctao_adapter -- <dataset_dir> [output_json]
```

### Examples

```bash
cargo run --bin ctao_adapter -- CTA-N
cargo run --bin ctao_adapter -- CTA-S
cargo run --bin ctao_adapter -- data/CTA-N data/CTA-N/scheduling_blocks.json
```

### Notes

- If `dataset_dir` is `CTA-N` or `CTA-S`, the script also checks `data/<dataset_dir>`.
- Default output is `<dataset_dir>/scheduling_blocks.json`.
- For current CTA datasets, each exported scheduling block contains exactly one task object.
