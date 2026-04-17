#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
COMPOSE_FILE="${REPO_ROOT}/webapp/docker/docker-compose.yml"

if [[ ! -f "${COMPOSE_FILE}" ]]; then
  echo "compose file not found: ${COMPOSE_FILE}" >&2
  exit 1
fi

cd "${REPO_ROOT}"

if [[ "${1:-}" == "down" ]]; then
  shift
  exec docker compose -f webapp/docker/docker-compose.yml down "$@"
fi

exec docker compose -f webapp/docker/docker-compose.yml up --build "$@"
