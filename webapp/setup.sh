#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
COMPOSE_FILE="${REPO_ROOT}/webapp/docker/docker-compose.yml"

resolve_host() {
  local host="$1"
  getent ahostsv4 "${host}" >/dev/null 2>&1 || getent hosts "${host}" >/dev/null 2>&1
}

check_registry_dns() {
  local -a missing_hosts=()
  local host

  # Docker always needs to resolve Docker Hub when pulling base images.
  for host in registry-1.docker.io; do
    if ! resolve_host "${host}"; then
      missing_hosts+=("${host}")
    fi
  done

  # If a mirror is configured, ensure it resolves too so build errors are obvious.
  if [[ -r /etc/docker/daemon.json ]]; then
    local mirror_host
    mirror_host="$(sed -n 's/.*https:\/\/\([^\"\/]*\).*/\1/p' /etc/docker/daemon.json | head -n1)"
    if [[ -n "${mirror_host}" ]] && ! resolve_host "${mirror_host}"; then
      missing_hosts+=("${mirror_host}")
    fi
  fi

  if (( ${#missing_hosts[@]} > 0 )); then
    echo "Docker registry DNS preflight failed." >&2
    echo "Could not resolve required host(s):" >&2
    for host in "${missing_hosts[@]}"; do
      echo "  - ${host}" >&2
    done
    echo >&2
    if command -v nslookup >/dev/null 2>&1 \
      && nslookup registry-1.docker.io 8.8.8.8 >/dev/null 2>&1; then
      echo "Public DNS can resolve Docker Hub, so your local resolver path is likely the problem." >&2
      echo >&2
    fi
    echo "Suggested fix:" >&2
    echo "  1) Ensure your host resolver can resolve registry domains." >&2
    echo "  2) Remove unreachable registry mirrors from /etc/docker/daemon.json if present." >&2
    echo "  3) Restart resolver and Docker: sudo systemctl restart systemd-resolved docker" >&2
    echo "  4) Verify: docker pull rust:latest" >&2
    echo >&2
    echo "Set PHD_SKIP_DNS_PREFLIGHT=1 to bypass this check." >&2
    exit 1
  fi
}

if [[ ! -f "${COMPOSE_FILE}" ]]; then
  echo "compose file not found: ${COMPOSE_FILE}" >&2
  exit 1
fi

cd "${REPO_ROOT}"

if [[ "${1:-}" == "down" ]]; then
  shift
  exec docker compose -f webapp/docker/docker-compose.yml down "$@"
fi

if [[ "${PHD_SKIP_DNS_PREFLIGHT:-0}" != "1" ]]; then
  check_registry_dns
fi

exec docker compose -f webapp/docker/docker-compose.yml up --build "$@"
