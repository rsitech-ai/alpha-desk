#!/usr/bin/env bash
set -euo pipefail

readonly POSTGRES_IMAGE='docker.io/library/postgres:18.4-alpine3.24@sha256:9a8afca54e7861fd90fab5fdf4c42477a6b1cb7d293595148e674e0a3181de15'
readonly MIGRATION='schemas/postgres/0001_capture_incidents.sql'
readonly CONTAINER_NAME="alpha-desk-postgres-migration-$$"

for command_name in docker sleep; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf 'postgres-migration-smoke:error required command is unavailable: %s\n' \
      "$command_name" >&2
    exit 2
  fi
done

if [[ ! -f "$MIGRATION" || -L "$MIGRATION" ]]; then
  printf 'postgres-migration-smoke:error migration must be a regular non-symlink file: %s\n' \
    "$MIGRATION" >&2
  exit 2
fi

cleanup() {
  local status=$?
  docker rm --force "$CONTAINER_NAME" >/dev/null 2>&1 || true
  exit "$status"
}
trap cleanup EXIT INT TERM

docker run \
  --detach \
  --pull never \
  --name "$CONTAINER_NAME" \
  --network none \
  --read-only \
  --user 70:70 \
  --cap-drop ALL \
  --security-opt no-new-privileges:true \
  --env POSTGRES_USER=alpha \
  --env POSTGRES_PASSWORD=alpha_dev_only \
  --env POSTGRES_DB=alpha \
  --env PGDATA=/var/lib/postgresql/18/docker \
  --env POSTGRES_INITDB_ARGS='--no-locale --auth-local=scram-sha-256' \
  --tmpfs /var/lib/postgresql:rw,noexec,nosuid,nodev,size=256m,uid=70,gid=70 \
  --tmpfs /tmp:rw,noexec,nosuid,nodev,size=32m,uid=70,gid=70 \
  --tmpfs /var/run/postgresql:rw,noexec,nosuid,nodev,size=16m,uid=70,gid=70 \
  "$POSTGRES_IMAGE" >/dev/null

ready=false
for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12; do
  if docker exec "$CONTAINER_NAME" \
    pg_isready --username alpha --dbname alpha >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 1
done
if [[ "$ready" != true ]]; then
  docker logs "$CONTAINER_NAME" >&2
  printf 'postgres-migration-smoke:error PostgreSQL did not become ready\n' >&2
  exit 1
fi

docker exec --env PGPASSWORD=alpha_dev_only -i "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
  <"$MIGRATION" >/dev/null

actual_tables="$(
  docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
    psql -X --no-psqlrc --tuples-only --no-align \
    --username alpha --dbname alpha \
    --command \
    "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_name LIKE 'capture_%' ORDER BY table_name;"
)"
readonly actual_tables
readonly expected_tables=$'capture_incident_evidence\ncapture_incidents\ncapture_sequencer_cursors'
if [[ "$actual_tables" != "$expected_tables" ]]; then
  printf 'postgres-migration-smoke:error unexpected table set\n%s\n' \
    "$actual_tables" >&2
  exit 1
fi

if docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
  --command \
  "INSERT INTO capture_sequencer_cursors (chain_id, committed_block_height, canonical_block_hash, archive_manifest_hash, archive_receipt_id, cursor_version, updated_at) VALUES ('mainnet', 1, decode('00', 'hex'), decode(repeat('00', 32), 'hex'), 'receipt', 1, now());" \
  >/dev/null 2>&1; then
  printf 'postgres-migration-smoke:error invalid canonical hash length was accepted\n' >&2
  exit 1
fi

printf 'postgres-migration-smoke:ok image=%s\n' "$POSTGRES_IMAGE"
