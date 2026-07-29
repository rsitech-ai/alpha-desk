#!/bin/sh
set -eu

nats_url="${NATS_URL:-nats://nats:4222}"
policy_path="${NATS_BOOTSTRAP_POLICY:-/opt/alpha-desk/bootstrap.json}"
nats_user="${NATS_BOOTSTRAP_USER:-}"
nats_password="${NATS_BOOTSTRAP_PASSWORD:-}"

test -f "$policy_path" || {
  printf 'nats-streams:error missing bootstrap policy\n' >&2
  exit 1
}
test -n "$nats_user" && test -n "$nats_password" || {
  printf 'nats-streams:error missing bootstrap credentials\n' >&2
  exit 1
}

nats_admin() {
  nats \
    --server "$nats_url" \
    --user "$nats_user" \
    --password "$nats_password" \
    "$@"
}

jq -e '
  .schema_version == 1 and
  .environment == "development" and
  .policy.storage == "file" and
  .policy.retention == "limits" and
  .policy.discard == "old" and
  (.policy.max_age_seconds >= 21600 and .policy.max_age_seconds <= 86400) and
  (.policy.max_bytes > 0) and
  (.policy.max_messages > 0) and
  (.policy.max_message_bytes > 0 and .policy.max_message_bytes <= 7500000) and
  (.policy.max_consumers > 0) and
  (.policy.duplicate_window_seconds >= 600) and
  .policy.replicas == 1 and
  (.streams | length == 6) and
  ([.streams[].name] | unique | length) == 6 and
  all(.streams[]; (.subjects | length) > 0)
' "$policy_path" >/dev/null || {
  printf 'nats-streams:error invalid bootstrap policy\n' >&2
  exit 1
}

max_age_seconds="$(jq -r '.policy.max_age_seconds' "$policy_path")"
max_age_nanoseconds="$((max_age_seconds * 1000000000))"
max_bytes="$(jq -r '.policy.max_bytes' "$policy_path")"
max_messages="$(jq -r '.policy.max_messages' "$policy_path")"
max_message_bytes="$(jq -r '.policy.max_message_bytes' "$policy_path")"
max_consumers="$(jq -r '.policy.max_consumers' "$policy_path")"
duplicate_window_seconds="$(jq -r '.policy.duplicate_window_seconds' "$policy_path")"
replicas="$(jq -r '.policy.replicas' "$policy_path")"

configure_stream() {
  name="$1"
  subjects="$2"

  if nats_admin stream info "$name" >/dev/null 2>&1; then
    nats_admin stream edit "$name" \
      --subjects "$subjects" \
      --retention limits \
      --discard old \
      --max-age "${max_age_seconds}s" \
      --max-bytes "$max_bytes" \
      --max-msgs "$max_messages" \
      --max-msg-size "$max_message_bytes" \
      --max-consumers "$max_consumers" \
      --dupe-window "${duplicate_window_seconds}s" \
      --replicas "$replicas" \
      --force >/dev/null
  else
    nats_admin stream add "$name" \
      --subjects "$subjects" \
      --storage file \
      --retention limits \
      --discard old \
      --max-age "${max_age_seconds}s" \
      --max-bytes "$max_bytes" \
      --max-msgs "$max_messages" \
      --max-msg-size "$max_message_bytes" \
      --max-consumers "$max_consumers" \
      --dupe-window "${duplicate_window_seconds}s" \
      --replicas "$replicas" \
      --defaults >/dev/null
  fi

  stream_json="$(nats_admin stream info "$name" --json)"
  printf '%s\n' "$stream_json" |
    jq -e \
      --arg name "$name" \
      --arg subjects "$subjects" \
      --argjson max_age "$max_age_nanoseconds" \
      --argjson max_bytes "$max_bytes" \
      --argjson max_messages "$max_messages" \
      --argjson max_message_bytes "$max_message_bytes" \
      --argjson max_consumers "$max_consumers" \
      --argjson duplicate_window "$((duplicate_window_seconds * 1000000000))" \
      --argjson replicas "$replicas" '
        .config.name == $name and
        .config.storage == "file" and
        .config.retention == "limits" and
        .config.discard == "old" and
        .config.max_age == $max_age and
        .config.max_bytes == $max_bytes and
        .config.max_msgs == $max_messages and
        .config.max_msg_size == $max_message_bytes and
        .config.max_consumers == $max_consumers and
        .config.duplicate_window == $duplicate_window and
        .config.num_replicas == $replicas and
        (
          [.config.subjects[]] | sort
        ) == (
          $subjects | split(",") | sort
        )
      ' >/dev/null
}

jq -r '.streams[] | [.name, (.subjects | join(","))] | @tsv' "$policy_path" |
  while IFS="$(printf '\t')" read -r stream_name stream_subjects; do
    configure_stream "$stream_name" "$stream_subjects"
  done

printf 'nats-streams:ok\n'
