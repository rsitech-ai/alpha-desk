#!/usr/bin/env bash
set -euo pipefail

script_dir="$(
  CDPATH='' builtin cd -- "$(command dirname -- "${BASH_SOURCE[0]}")" &&
    builtin pwd -P
)"
repo_root="$(
  CDPATH='' builtin cd -- "$script_dir/../.." &&
    builtin pwd -P
)"
compose_files="${DEV_STACK_COMPOSE_FILES:-$repo_root/infra/docker-compose/compose.yaml}"
compose_project="${DEV_STACK_COMPOSE_PROJECT:-alpha-desk-dev}"
nats_monitor_port="${DEV_STACK_NATS_MONITOR_PORT:-8222}"
clickhouse_port="${DEV_STACK_CLICKHOUSE_PORT:-8123}"
postgres_port="${DEV_STACK_POSTGRES_PORT:-5432}"
minio_port="${DEV_STACK_MINIO_PORT:-9000}"
otel_health_port="${DEV_STACK_OTEL_HEALTH_PORT:-13133}"
victoriametrics_port="${DEV_STACK_VICTORIAMETRICS_PORT:-8428}"
IFS=':' read -r -a compose_file_list <<<"$compose_files"
compose=(docker compose --project-name "$compose_project")
for compose_file in "${compose_file_list[@]}"; do
  [[ -f "$compose_file" ]] || {
    printf 'dev-stack-wait:error missing Compose file %s\n' "$compose_file" >&2
    exit 2
  }
  compose+=(-f "$compose_file")
done

timeout_seconds="${DEV_STACK_WAIT_TIMEOUT_SECONDS:-120}"
poll_seconds="${DEV_STACK_WAIT_POLL_SECONDS:-2}"

[[ "$timeout_seconds" =~ ^[0-9]+$ ]] || {
  printf 'dev-stack-wait:error invalid timeout %q\n' "$timeout_seconds" >&2
  exit 2
}
[[ "$poll_seconds" =~ ^[0-9]+([.][0-9]+)?$ ]] || {
  printf 'dev-stack-wait:error invalid poll interval %q\n' "$poll_seconds" >&2
  exit 2
}

for command_name in curl docker jq pg_isready psql; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'dev-stack-wait:error missing command %s\n' "$command_name" >&2
    exit 2
  }
done

deadline=$((SECONDS + timeout_seconds))

check_initializer() {
  local service="$1"
  local container_output state_json
  local -a container_ids=()

  container_output="$(
    "${compose[@]}" ps --all --quiet "$service" 2>/dev/null
  )" || return 1

  [[ -n "$container_output" ]] || return 1

  while IFS= read -r container_id; do
    [[ "$container_id" =~ ^[0-9a-f]{64}$ ]] || return 1
    container_ids+=("$container_id")
  done <<<"$container_output"

  [[ "${#container_ids[@]}" -eq 1 ]] || return 1

  state_json="$(
    docker inspect \
      --type container \
      --format '{{json .State}}' \
      "${container_ids[0]}" 2>/dev/null
  )" || return 1

  printf '%s\n' "$state_json" |
    jq -s -e '
      length == 1 and
      .[0].Status == "exited" and
      .[0].Running == false and
      .[0].ExitCode == 0
    ' >/dev/null
}

check_nats() {
  check_initializer nats-init &&
    curl --fail --silent --show-error --max-time 2 \
      "http://127.0.0.1:${nats_monitor_port}/healthz?js-enabled-only=true" 2>/dev/null |
      jq -e '.status == "ok" and ((.error? // null) == null)' >/dev/null
}

check_clickhouse() {
  check_initializer clickhouse-init &&
    response="$(
      curl --fail --silent --show-error --max-time 2 \
        "http://127.0.0.1:${clickhouse_port}/ping" 2>/dev/null
    )" &&
    [[ "$response" == "Ok." ]]
}

check_postgres() {
  pg_isready -h 127.0.0.1 -p "$postgres_port" -U alpha -d alpha -t 2 >/dev/null 2>&1 &&
    PGPASSWORD=alpha_dev_only PGCONNECT_TIMEOUT=2 \
      psql -X --no-password --tuples-only --no-align \
      -h 127.0.0.1 -p "$postgres_port" -U alpha -d alpha \
      -c "SELECT current_database() || '|' || current_user" 2>/dev/null |
      grep -qx 'alpha|alpha'
}

check_minio() {
  check_initializer minio-init &&
    curl --fail --silent --show-error --max-time 2 --output /dev/null \
      "http://127.0.0.1:${minio_port}/minio/health/live" 2>/dev/null
}

check_otel() {
  curl --fail --silent --show-error --max-time 2 --output /dev/null \
    "http://127.0.0.1:${otel_health_port}/" 2>/dev/null
}

check_victoriametrics() {
  curl --fail --silent --show-error --max-time 2 --output /dev/null \
    "http://127.0.0.1:${victoriametrics_port}/health" 2>/dev/null &&
    curl --fail --silent --show-error --max-time 2 \
      "http://127.0.0.1:${victoriametrics_port}/api/v1/targets" 2>/dev/null |
      jq -e '
        .status == "success" and
        (.data.activeTargets | type == "array") and
        (all(.data.activeTargets[]; .health == "up")) and
        (
          [.data.activeTargets[].labels.job] | sort
        ) == (
          [
            "alpha-desk-otlp-metrics",
            "otel-collector-internal",
            "victoriametrics"
          ] | sort
        )
      ' >/dev/null
}

wait_for() {
  label="$1"
  check_function="$2"

  while true; do
    if "$check_function"; then
      printf '%s:ok\n' "$label"
      return 0
    fi

    if ((SECONDS >= deadline)); then
      printf '%s:error timeout\n' "$label" >&2
      return 1
    fi

    sleep "$poll_seconds"
  done
}

wait_for nats check_nats
wait_for clickhouse check_clickhouse
wait_for postgres check_postgres
wait_for minio check_minio
wait_for otel check_otel
wait_for victoriametrics check_victoriametrics
