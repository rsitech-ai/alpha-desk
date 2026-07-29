#!/bin/sh
set -eu

nats_url="${NATS_URL:-nats://nats:4222}"

for variable_name in \
  NATS_BOOTSTRAP_USER \
  NATS_BOOTSTRAP_PASSWORD \
  NATS_CAPTURE_USER \
  NATS_CAPTURE_PASSWORD \
  NATS_READER_USER \
  NATS_READER_PASSWORD; do
  variable_value="$(printenv "$variable_name" || true)"
  test -n "$variable_value" || {
    printf 'nats-permissions:error missing %s\n' "$variable_name" >&2
    exit 2
  }
done

nats_as() {
  user="$1"
  password="$2"
  shift 2
  nats \
    --server "$nats_url" \
    --user "$user" \
    --password "$password" \
    --timeout 2s \
    "$@"
}

expect_denied() {
  label="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    printf 'nats-permissions:error %s unexpectedly succeeded\n' "$label" >&2
    exit 1
  fi
}

nats_as \
  "$NATS_BOOTSTRAP_USER" \
  "$NATS_BOOTSTRAP_PASSWORD" \
  stream info HL_CANONICAL --json >/dev/null

nats_as \
  "$NATS_CAPTURE_USER" \
  "$NATS_CAPTURE_PASSWORD" \
  publish --jetstream hl.v1.event.fill permission-probe >/dev/null

expect_denied \
  'capture state publish' \
  nats_as \
  "$NATS_CAPTURE_USER" \
  "$NATS_CAPTURE_PASSWORD" \
  publish --jetstream hl.v1.state.account_delta permission-probe

expect_denied \
  'capture stream administration' \
  nats_as \
  "$NATS_CAPTURE_USER" \
  "$NATS_CAPTURE_PASSWORD" \
  stream info HL_CANONICAL --json

expect_denied \
  'reader canonical publish' \
  nats_as \
  "$NATS_READER_USER" \
  "$NATS_READER_PASSWORD" \
  publish --jetstream hl.v1.event.fill permission-probe

nats_as \
  "$NATS_READER_USER" \
  "$NATS_READER_PASSWORD" \
  stream info HL_CANONICAL --json >/dev/null

expect_denied \
  'anonymous access' \
  nats --server "$nats_url" --timeout 2s stream info HL_CANONICAL --json

printf '%s\n' 'nats-permissions:ok'
