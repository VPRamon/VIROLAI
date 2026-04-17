#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
COMPOSE_FILE="${REPO_ROOT}/webapp/docker/docker-compose.yml"

if [[ ! -f "${COMPOSE_FILE}" ]]; then
  echo "compose file not found: ${COMPOSE_FILE}" >&2
  exit 1
fi

usage() {
  cat <<'EOF'
Usage: ./webapp/teardown.sh [--purge-db] [docker-compose down args...]

Stops and removes the stack containers/network.
By default, the database volume is kept (persistent).

Options:
  --purge-db   Also remove Docker volumes (deletes PostgreSQL data)
  -h, --help   Show this help message
EOF
}

purge_db=false
args=()

while (($#)); do
  case "$1" in
    --purge-db)
      purge_db=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      args+=("$1")
      shift
      ;;
  esac
done

cd "${REPO_ROOT}"

cmd=(docker compose -f webapp/docker/docker-compose.yml down --remove-orphans)

if [[ "${purge_db}" == "true" ]]; then
  cmd+=(--volumes)
fi

exec "${cmd[@]}" "${args[@]}"
