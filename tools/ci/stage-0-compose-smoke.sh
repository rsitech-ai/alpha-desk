#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly repository_root
readonly compose_file="${repository_root}/infra/docker-compose/compose.yaml"
readonly override_file="${repository_root}/infra/docker-compose/stage-0.override.yaml"
nonce_directory="$(mktemp -d "${TMPDIR:-/tmp}/alpha-desk-stage0.XXXXXXXX")"
readonly nonce_directory
nonce="${nonce_directory##*.}"
[[ "${nonce}" =~ ^[A-Za-z0-9]{8}$ ]] || {
  printf '%s\n' 'stage-0-compose:error invalid gate nonce' >&2
  exit 2
}
readonly project="alpha-desk-stage0-${nonce}"
readonly merged_config="${nonce_directory}/merged.json"
export STAGE_GATE_COMPOSE_PROJECT="${project}"
compose=(docker compose --project-name "${project}" -f "${compose_file}" -f "${override_file}")
started_by_gate=false

cleanup() {
  if [[ "${started_by_gate}" == true ]]; then
    "${compose[@]}" down --timeout 60 --volumes --remove-orphans
    [[ -z "$("${compose[@]}" ps --all --quiet)" ]] || {
      printf '%s\n' 'stage-0-compose:error owned containers remain after cleanup' >&2
      return 1
    }
    for resource in "${owned_resources[@]}"; do
      if docker volume inspect "${resource}" >/dev/null 2>&1; then
        printf 'stage-0-compose:error owned volume remains: %s\n' "${resource}" >&2
        return 1
      fi
    done
    if docker network inspect "${project}_network" >/dev/null 2>&1; then
      printf 'stage-0-compose:error owned network remains: %s\n' "${project}_network" >&2
      return 1
    fi
    started_by_gate=false
  fi
}
finish() {
  local status=$?
  cleanup || true
  rm -f -- "${merged_config}"
  rmdir -- "${nonce_directory}" 2>/dev/null || true
  return "${status}"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

cd "${repository_root}"
./infra/docker-compose/test-contract.sh

owned_resources=(
  "${project}_nats-data"
  "${project}_clickhouse-data"
  "${project}_postgres-data"
  "${project}_minio-data"
  "${project}_victoriametrics-data"
)
for resource in "${owned_resources[@]}"; do
  if docker volume inspect "${resource}" >/dev/null 2>&1; then
    printf 'stage-0-compose-blocked: volume already exists: %s\n' "${resource}" >&2
    exit 2
  fi
done
if docker network inspect "${project}_network" >/dev/null 2>&1; then
  printf 'stage-0-compose-blocked: network already exists: %s\n' "${project}_network" >&2
  exit 2
fi
if [[ -n "$("${compose[@]}" ps --all --quiet)" ]]; then
  printf '%s\n' 'stage-0-compose-blocked: unique project unexpectedly has containers' >&2
  exit 2
fi

"${compose[@]}" config --format json >"${merged_config}"
jq -e --arg project "${project}" '
  .name == $project and
  .volumes == {
    "clickhouse-data": {"name": ($project + "_clickhouse-data")},
    "minio-data": {"name": ($project + "_minio-data")},
    "nats-data": {"name": ($project + "_nats-data")},
    "postgres-data": {"name": ($project + "_postgres-data")},
    "victoriametrics-data": {"name": ($project + "_victoriametrics-data")}
  } and
  .networks == {"default": {"name": ($project + "_network")}} and
  ([.services.nats.ports[].published] | sort) == ["14222", "18222"] and
  [.services.clickhouse.ports[].published] == ["18123"] and
  [.services.postgres.ports[].published] == ["15432"] and
  [.services.minio.ports[].published] == ["19000"] and
  ([.services["otel-collector"].ports[].published] | sort) == ["13134", "14317", "14318"] and
  [.services.victoriametrics.ports[].published] == ["18428"]
' "${merged_config}" >/dev/null

started_by_gate=true
"${compose[@]}" up -d --wait --wait-timeout 120
DEV_STACK_COMPOSE_PROJECT="${project}" \
DEV_STACK_COMPOSE_FILES="${compose_file}:${override_file}" \
DEV_STACK_NATS_MONITOR_PORT=18222 \
DEV_STACK_CLICKHOUSE_PORT=18123 \
DEV_STACK_POSTGRES_PORT=15432 \
DEV_STACK_MINIO_PORT=19000 \
DEV_STACK_OTEL_HEALTH_PORT=13134 \
DEV_STACK_VICTORIAMETRICS_PORT=18428 \
  ./tools/ci/wait-for-dev-stack.sh
cleanup
