# Adapted TSI Docker Setup

This directory contains the Docker assets for running TSI with the PhD schema adapter backend.

## Files

- docker-compose.yml: frontend + adapted backend stack
- Dockerfile.backend: builds the phd_tsi_server binary
- Dockerfile.frontend: builds and serves the TSI frontend
- Dockerfile.backend.dockerignore: backend-specific Docker ignore rules
- Dockerfile.frontend.dockerignore: frontend-specific Docker ignore rules
- ../setup.sh: helper script to start/stop the stack

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
./webapp/setup.sh down
```

Follow logs:

```bash
docker compose -f webapp/docker/docker-compose.yml logs -f
```

## Optional Environment Variables

- BACKEND_PORT (default: 8080)
- FRONTEND_PORT (default: 3000)
- RUST_LOG (default: info)
