# Adapted TSI Docker Setup

This directory contains the Docker assets for running TSI with the PhD schema adapter backend.

## Files

- docker-compose.yml: frontend + adapted backend + postgres stack
- Dockerfile.backend: builds the phd_tsi_server binary
- Dockerfile.frontend: builds and serves the TSI frontend
- Dockerfile.backend.dockerignore: backend-specific Docker ignore rules
- Dockerfile.frontend.dockerignore: frontend-specific Docker ignore rules
- ../setup.sh: helper script to start the stack
- ../teardown.sh: helper script to stop/remove the stack

## Quick Start

From the repository root:

```bash
./webapp/setup.sh
```

This runs:

```bash
docker compose -f webapp/docker/docker-compose.yml up --build
```

Services:

- Frontend: http://localhost:3000
- Backend health: http://localhost:8080/health

## Useful Commands

Detached mode:

```bash
./webapp/setup.sh -d
```

Stop services:

```bash
./webapp/teardown.sh
```

Stop services and delete database data:

```bash
./webapp/teardown.sh --purge-db
```

Follow logs:

```bash
docker compose -f webapp/docker/docker-compose.yml logs -f
```

## Optional Environment Variables

- BACKEND_PORT (default: 8080)
- FRONTEND_PORT (default: 3000)
- POSTGRES_PORT (default: 5432)
- POSTGRES_USER (default: tsi)
- POSTGRES_PASSWORD (default: tsi)
- POSTGRES_DB (default: tsi)
- RUST_LOG (default: info)

## Database Persistence

PostgreSQL data is stored in the named Docker volume `postgres_data`, so data survives restarts and normal teardown (`./webapp/teardown.sh`).
