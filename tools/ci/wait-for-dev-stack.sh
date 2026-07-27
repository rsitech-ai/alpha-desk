#!/usr/bin/env bash
set -euo pipefail

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

for command_name in curl jq pg_isready psql; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'dev-stack-wait:error missing command %s\n' "$command_name" >&2
    exit 2
  }
done

deadline=$((SECONDS + timeout_seconds))

check_nats() {
  curl --fail --silent --show-error --max-time 2 \
    'http://127.0.0.1:8222/healthz?js-enabled-only=true' 2>/dev/null |
    jq -e '.status == "ok" and ((.error? // null) == null)' >/dev/null
}

check_clickhouse() {
  response="$(
    curl --fail --silent --show-error --max-time 2 \
      http://127.0.0.1:8123/ping 2>/dev/null
  )" &&
    [[ "$response" == "Ok." ]]
}

check_postgres() {
  pg_isready -h 127.0.0.1 -p 5432 -U alpha -d alpha -t 2 >/dev/null 2>&1 &&
    PGPASSWORD=alpha_dev_only PGCONNECT_TIMEOUT=2 \
      psql -X --no-password --tuples-only --no-align \
      -h 127.0.0.1 -p 5432 -U alpha -d alpha \
      -c "SELECT current_database() || '|' || current_user" 2>/dev/null |
      grep -qx 'alpha|alpha'
}

check_minio() {
  curl --fail --silent --show-error --max-time 2 --output /dev/null \
    http://127.0.0.1:9000/minio/health/live 2>/dev/null
}

check_otel() {
  curl --fail --silent --show-error --max-time 2 --output /dev/null \
    http://127.0.0.1:13133/ 2>/dev/null
}

check_victoriametrics() {
  curl --fail --silent --show-error --max-time 2 --output /dev/null \
    http://127.0.0.1:8428/health 2>/dev/null &&
    curl --fail --silent --show-error --max-time 2 \
      http://127.0.0.1:8428/api/v1/targets 2>/dev/null |
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
