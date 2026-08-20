# VIROLAI-TSI Docker setup

This directory contains Docker assets for running the TSI frontend with the VIROLAI integration backend and PostgreSQL.

## Files

- `docker-compose.yml`: frontend, backend, and PostgreSQL stack
- `Dockerfile.backend`: builds the VIROLAI integration backend
- `Dockerfile.frontend`: builds and serves the TSI frontend
- `../setup.sh`: starts the stack
- `../teardown.sh`: stops the stack

## Start

From the repository root:

```bash
./webapp/setup.sh
```

Detached mode:

```bash
./webapp/setup.sh -d
```

Default endpoints:

- frontend: `http://localhost:3000`
- backend health: `http://localhost:8080/health`

## Stop

```bash
./webapp/teardown.sh
```

Delete the database volume as well:

```bash
./webapp/teardown.sh --purge-db
```

## Configuration

Common variables:

- `BACKEND_PORT`, default `8080`
- `FRONTEND_PORT`, default `3000`
- `POSTGRES_PORT`, default `5432`
- `POSTGRES_USER`, default `tsi`
- `POSTGRES_PASSWORD`, default `tsi`
- `POSTGRES_DB`, default `tsi`
- `RUST_LOG`, default `info`
- `VIROLAI_WORKSPACES_DIR`, default `./workspaces` outside Docker

The backend still accepts `PHD_WORKSPACES_DIR` as a compatibility fallback.

Set `VIROLAI_SKIP_DNS_PREFLIGHT=1` to bypass the registry DNS check in `webapp/setup.sh`. The former `PHD_SKIP_DNS_PREFLIGHT` name is also accepted as a fallback.

## Database persistence

PostgreSQL data is stored in the `postgres_data` Docker volume, so normal teardown does not remove database contents.

## Registry DNS failures

If setup fails before the build with a Docker registry name-resolution error, verify that the host can resolve Docker Hub and any configured registry mirror:

```bash
getent hosts registry-1.docker.io
docker pull rust:latest
```

If public DNS works but Docker does not, check the host resolver configuration and `/etc/docker/daemon.json`, then restart the resolver and Docker services.
