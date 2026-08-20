# `scripts/`

Operational helpers for VIROLAI and the webapp.

| Script | Purpose |
| --- | --- |
| `qa-pipeline.sh` | Run workspace formatting, clippy, and tests |
| `upload_results.sh` | Compatibility wrapper around `virolai publish` |
| `webapp` | Local webapp backend helper |

The CTAO converter is provided as the `lab-ctao-adapter` binary in the `lab` crate. It is an optional dataset integration rather than part of the scheduler core.

## QA pipeline

```bash
./scripts/qa-pipeline.sh
```

This runs the checks specified in `AGENTS.md` and stops on the first failure.

## CTAO adapter

Convert a supported CTAO source dataset into the common scheduling problem model:

```bash
cargo run -p lab --bin lab-ctao-adapter -- \
  --input data/raw/cta_n_internalSDC.json \
  --output data/cta_n.json
```

The output is consumed by the same scheduler interfaces as any other `scheduling_problem.json` input.

## `upload_results.sh`

This script exists for compatibility with shell pipelines. New callers should invoke the Rust CLI directly:

```bash
cargo run -p lab --bin virolai -- publish \
  --workspace paper \
  --dir out/results \
  --create-workspace
```

The wrapper locates a built `virolai` binary or uses the one available on `PATH`.

## Webapp

Run the backend locally with:

```bash
VIROLAI_WORKSPACES_DIR=./workspaces cargo run -p webapp --bin webapp
```

`PHD_WORKSPACES_DIR` remains accepted as a compatibility fallback.

For the Docker stack, use `webapp/setup.sh` and the files under `webapp/docker/`.
