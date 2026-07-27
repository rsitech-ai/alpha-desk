#!/bin/sh
set -eu

nats_url="${NATS_URL:-nats://nats:4222}"
max_age_nanoseconds=21600000000000
max_bytes=536870912

configure_stream() {
  name="$1"
  subjects="$2"

  if nats --server "$nats_url" stream info "$name" >/dev/null 2>&1; then
    nats --server "$nats_url" stream edit "$name" \
      --subjects "$subjects" \
      --retention limits \
      --discard old \
      --max-age 6h \
      --max-bytes "$max_bytes" \
      --replicas 1 \
      --force >/dev/null
  else
    nats --server "$nats_url" stream add "$name" \
      --subjects "$subjects" \
      --storage file \
      --retention limits \
      --discard old \
      --max-age 6h \
      --max-bytes "$max_bytes" \
      --replicas 1 \
      --defaults >/dev/null
  fi

  stream_json="$(nats --server "$nats_url" stream info "$name" --json)"
  printf '%s\n' "$stream_json" |
    jq -e \
      --arg name "$name" \
      --arg subjects "$subjects" \
      --argjson max_age "$max_age_nanoseconds" \
      --argjson max_bytes "$max_bytes" '
        .config.name == $name and
        .config.storage == "file" and
        .config.retention == "limits" and
        .config.discard == "old" and
        .config.max_age == $max_age and
        .config.max_bytes == $max_bytes and
        .config.num_replicas == 1 and
        (
          [.config.subjects[]] | sort
        ) == (
          $subjects | split(",") | sort
        )
      ' >/dev/null
}

configure_stream HL_CANONICAL 'hl.v1.block.*,hl.v1.event.*'
configure_stream HL_STATE 'hl.v1.state.*'
configure_stream HL_FEATURE 'hl.v1.feature.*'
configure_stream HL_SIGNAL 'hl.v1.signal.*'
configure_stream HL_HEALTH 'hl.v1.health.*'
configure_stream HL_DEADLETTER 'hl.v1.deadletter.>'

printf 'nats-streams:ok\n'
