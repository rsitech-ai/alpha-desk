#!/usr/bin/env bash
set -euo pipefail

readonly POSTGRES_IMAGE='docker.io/library/postgres:18.4-alpine3.24@sha256:9a8afca54e7861fd90fab5fdf4c42477a6b1cb7d293595148e674e0a3181de15'
readonly CONTAINER_NAME="alpha-desk-postgres-migration-$$"
readonly -a MIGRATIONS=(
  'schemas/postgres/0001_capture_incidents.sql'
  'schemas/postgres/0002_capture_progress.sql'
)

for command_name in docker sleep; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf 'postgres-migration-smoke:error required command is unavailable: %s\n' \
      "$command_name" >&2
    exit 2
  fi
done

for migration in "${MIGRATIONS[@]}"; do
  if [[ ! -f "$migration" || -L "$migration" ]]; then
    printf 'postgres-migration-smoke:error migration must be a regular non-symlink file: %s\n' \
      "$migration" >&2
    exit 2
  fi
done

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

docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  createdb --username alpha capture_migration_legacy
docker exec --env PGPASSWORD=alpha_dev_only -i "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 \
  --username alpha --dbname capture_migration_legacy \
  <"${MIGRATIONS[0]}" >/dev/null
docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 \
  --username alpha --dbname capture_migration_legacy \
  --command "
    INSERT INTO capture_sequencer_cursors (
      chain_id,
      committed_block_height,
      canonical_block_hash,
      archive_manifest_hash,
      archive_receipt_id,
      cursor_version,
      updated_at
    ) VALUES (
      'legacy',
      1,
      decode(repeat('01', 32), 'hex'),
      decode(repeat('02', 32), 'hex'),
      'receipt',
      1,
      now()
    );
  " >/dev/null
if docker exec --env PGPASSWORD=alpha_dev_only -i "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 \
  --username alpha --dbname capture_migration_legacy \
  <"${MIGRATIONS[1]}" >/dev/null 2>&1; then
  printf 'postgres-migration-smoke:error legacy cursor preflight did not fail closed\n' >&2
  exit 1
fi
docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  dropdb --username alpha capture_migration_legacy

for migration in "${MIGRATIONS[@]}"; do
  docker exec --env PGPASSWORD=alpha_dev_only -i "$CONTAINER_NAME" \
    psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
    <"$migration" >/dev/null
done

actual_tables="$(
  docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
    psql -X --no-psqlrc --tuples-only --no-align \
    --username alpha --dbname alpha \
    --command \
    "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_name LIKE 'capture_%' ORDER BY table_name;"
)"
readonly actual_tables
readonly expected_tables=$'capture_archived_blocks\ncapture_block_publications\ncapture_chain_progress\ncapture_incident_evidence\ncapture_incidents\ncapture_sequencer_cursors'
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

docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
  --command "
    BEGIN;
    INSERT INTO capture_chain_progress (
      chain_id,
      first_block_height,
      initialized_at_micros
    ) VALUES (
      'u64-boundary',
      18446744073709551615,
      1721779300000000
    );
    INSERT INTO capture_archived_blocks (
      chain_id,
      block_height,
      canonical_block_hash,
      archive_receipt_id,
      archive_manifest_id,
      archive_object_hash,
      archive_manifest_hash,
      archive_schema_fingerprint,
      publication_count,
      state,
      archived_at_micros
    ) VALUES (
      'u64-boundary',
      18446744073709551615,
      decode(repeat('01', 32), 'hex'),
      'receipt',
      'manifest',
      decode(repeat('02', 32), 'hex'),
      decode(repeat('03', 32), 'hex'),
      decode(repeat('04', 32), 'hex'),
      1,
      'acknowledged',
      1721779300000000
    );
    INSERT INTO capture_block_publications (
      chain_id,
      block_height,
      publication_ordinal,
      message_id,
      subject,
      publication_hash,
      ack_stream,
      ack_stream_sequence,
      ack_duplicate,
      acknowledged_at_micros
    ) VALUES (
      'u64-boundary',
      18446744073709551615,
      0,
      'message',
      'hl.v1.block.committed',
      decode(repeat('03', 32), 'hex'),
      'HL_CANONICAL',
      18446744073709551615,
      false,
      1721779300000100
    );
    INSERT INTO capture_sequencer_cursors (
      chain_id,
      committed_block_height,
      canonical_block_hash,
      archive_manifest_hash,
      archive_receipt_id,
      cursor_version,
      updated_at,
      updated_at_micros
    ) VALUES (
      'u64-boundary',
      18446744073709551615,
      decode(repeat('01', 32), 'hex'),
      decode(repeat('03', 32), 'hex'),
      'receipt',
      18446744073709551615,
      to_timestamp(1721779300000100::double precision / 1000000),
      1721779300000100
    );
    COMMIT;
  " >/dev/null

if docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
  --command "
    INSERT INTO capture_chain_progress (
      chain_id,
      first_block_height,
      initialized_at_micros
    ) VALUES ('overflow', 18446744073709551616, 1);
  " >/dev/null 2>&1; then
  printf 'postgres-migration-smoke:error u64 overflow was accepted\n' >&2
  exit 1
fi

if docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
  --command "
    BEGIN;
    INSERT INTO capture_chain_progress (
      chain_id,
      first_block_height,
      initialized_at_micros
    ) VALUES ('incomplete', 1, 1);
    INSERT INTO capture_archived_blocks (
      chain_id,
      block_height,
      canonical_block_hash,
      archive_receipt_id,
      archive_manifest_id,
      archive_object_hash,
      archive_manifest_hash,
      archive_schema_fingerprint,
      publication_count,
      state,
      archived_at_micros
    ) VALUES (
      'incomplete',
      1,
      decode(repeat('01', 32), 'hex'),
      'receipt',
      'manifest',
      decode(repeat('02', 32), 'hex'),
      decode(repeat('03', 32), 'hex'),
      decode(repeat('04', 32), 'hex'),
      2,
      'archived_pending',
      1
    );
    INSERT INTO capture_block_publications (
      chain_id,
      block_height,
      publication_ordinal,
      message_id,
      subject,
      publication_hash
    ) VALUES (
      'incomplete',
      1,
      0,
      'message',
      'hl.v1.block.committed',
      decode(repeat('03', 32), 'hex')
    );
    COMMIT;
  " >/dev/null 2>&1; then
  printf 'postgres-migration-smoke:error incomplete publication plan was accepted\n' >&2
  exit 1
fi

printf 'postgres-migration-smoke:ok image=%s\n' "$POSTGRES_IMAGE"
