#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly repository_root
readonly compose_file="${repository_root}/infra/docker-compose/compose.yaml"
started_by_gate=false

cleanup() {
  if [[ "${started_by_gate}" == true ]]; then
    docker compose -f "${compose_file}" down --timeout 60 --remove-orphans
    started_by_gate=false
  fi
}
trap cleanup EXIT INT TERM

cd "${repository_root}"
./infra/docker-compose/test-contract.sh

if [[ -n "$(docker compose -f "${compose_file}" ps --all --quiet)" ]]; then
  printf '%s\n' "stage-0-compose-blocked: project already has containers; refusing to disturb an existing stack" >&2
  exit 2
fi

started_by_gate=true
docker compose -f "${compose_file}" up -d --wait --wait-timeout 120
./tools/ci/wait-for-dev-stack.sh
cleanup
