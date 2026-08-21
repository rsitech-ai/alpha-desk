#!/usr/bin/env bash
set -euo pipefail

readonly POSTGRES_IMAGE='docker.io/library/postgres:18.4-alpine3.24@sha256:9a8afca54e7861fd90fab5fdf4c42477a6b1cb7d293595148e674e0a3181de15'
readonly CONTAINER_NAME="alpha-desk-postgres-migration-$$"
readonly -a MIGRATIONS=(
  'schemas/postgres/0001_capture_incidents.sql'
  'schemas/postgres/0002_capture_progress.sql'
  'schemas/postgres/0100_source_catalog.sql'
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

actual_source_tables="$(
  docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
    psql -X --no-psqlrc --tuples-only --no-align \
    --username alpha --dbname alpha \
    --command \
    "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_name LIKE 'source_%' ORDER BY table_name;"
)"
readonly actual_source_tables
readonly expected_source_tables=$'source_capability_binding\nsource_endpoint_version\nsource_health_policy\nsource_license_policy\nsource_probe_result\nsource_registry'
if [[ "$actual_source_tables" != "$expected_source_tables" ]]; then
  printf 'postgres-migration-smoke:error unexpected source catalog table set\n%s\n' \
    "$actual_source_tables" >&2
  exit 1
fi

secret_columns="$(
  docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
    psql -X --no-psqlrc --tuples-only --no-align \
    --username alpha --dbname alpha \
    --command \
    "SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name LIKE 'source_%' AND (column_name ILIKE '%secret%' OR column_name ILIKE '%password%' OR column_name ILIKE '%api_key%' OR column_name ILIKE '%token%' OR column_name ILIKE '%credential%') ORDER BY column_name;"
)"
if [[ -n "$secret_columns" ]]; then
  printf 'postgres-migration-smoke:error source catalog must not store provider secrets\n%s\n' \
    "$secret_columns" >&2
  exit 1
fi

if docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
  --command "
    INSERT INTO source_registry (
      source_id, network, version, role, trust, operator, operator_kind,
      dataset_version, retention_class, redistribution, evidence_class,
      valid_from
    ) VALUES (
      'primary-node', 'mainnet', 1, 'committed-primary', 'locally-verified-committed',
      'alpha-desk', 'local-node', 'hyperliquid-node-v1', 'raw-indefinite',
      'private-operator-evidence', 'snapshot', now()
    );
  " >/dev/null 2>&1; then
  printf 'postgres-migration-smoke:error committed source without committed-block evidence was accepted\n' >&2
  exit 1
fi

if docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
  --command "
    INSERT INTO source_registry (
      source_id, network, version, role, trust, operator, operator_kind,
      retention_class, redistribution, evidence_class, valid_from
    ) VALUES (
      'nansen-labels', 'mainnet', 1, 'attribution-enrichment', 'third-party-provisional',
      'nansen', 'provider', 'raw-hot-local', 'internal-only', 'public-market-data', now()
    );
  " >/dev/null 2>&1; then
  printf 'postgres-migration-smoke:error provider source without license policy was accepted\n' >&2
  exit 1
fi

docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
  --command "
    BEGIN;
    INSERT INTO source_registry (
      source_id, network, version, role, trust, operator, operator_kind,
      dataset_version, retention_class, redistribution, evidence_class,
      valid_from
    ) VALUES (
      'primary-node', 'mainnet', 1, 'committed-primary', 'locally-verified-committed',
      'alpha-desk', 'local-node', 'hyperliquid-node-v1', 'raw-indefinite',
      'private-operator-evidence', 'committed-block', TIMESTAMPTZ '2026-01-01 00:00:00+00'
    );
    INSERT INTO source_registry (
      source_id, network, version, role, trust, operator, operator_kind,
      dataset_version, retention_class, redistribution, evidence_class,
      valid_from
    ) VALUES (
      'primary-node', 'testnet', 1, 'committed-primary', 'locally-verified-committed',
      'alpha-desk', 'local-node', 'hyperliquid-node-v1', 'raw-indefinite',
      'private-operator-evidence', 'committed-block', TIMESTAMPTZ '2026-01-01 00:00:00+00'
    );
    UPDATE source_registry
      SET valid_to = TIMESTAMPTZ '2026-02-01 00:00:00+00'
      WHERE source_id = 'primary-node' AND network = 'mainnet' AND version = 1;
    INSERT INTO source_registry (
      source_id, network, version, role, trust, operator, operator_kind,
      dataset_version, retention_class, redistribution, evidence_class,
      valid_from
    ) VALUES (
      'primary-node', 'mainnet', 2, 'committed-primary', 'locally-verified-committed',
      'alpha-desk', 'local-node', 'hyperliquid-node-v2', 'raw-indefinite',
      'private-operator-evidence', 'committed-block', TIMESTAMPTZ '2026-02-01 00:00:00+00'
    );
    INSERT INTO source_registry (
      source_id, network, version, role, trust, operator, operator_kind,
      retention_class, redistribution, evidence_class,
      license_name, agreement_status, agreement_expires_at, valid_from
    ) VALUES (
      'nansen-active', 'mainnet', 1, 'attribution-enrichment', 'third-party-provisional',
      'nansen', 'provider', 'raw-hot-local', 'internal-only', 'public-market-data',
      'nansen-api-tos', 'active', TIMESTAMPTZ '2026-12-01 00:00:00+00',
      TIMESTAMPTZ '2026-01-01 00:00:00+00'
    );
    INSERT INTO source_license_policy (
      source_id, network, source_version, license_name, redistribution
    ) VALUES (
      'nansen-active', 'mainnet', 1, 'nansen-api-tos', 'internal-only'
    );
    INSERT INTO source_registry (
      source_id, network, version, role, trust, operator, operator_kind,
      retention_class, redistribution, evidence_class,
      license_name, agreement_status, valid_from
    ) VALUES (
      'nansen-disabled', 'mainnet', 1, 'attribution-enrichment', 'third-party-provisional',
      'nansen', 'provider', 'raw-hot-local', 'internal-only', 'public-market-data',
      'nansen-api-tos', 'disabled', TIMESTAMPTZ '2026-01-01 00:00:00+00'
    );
    INSERT INTO source_registry (
      source_id, network, version, role, trust, operator, operator_kind,
      retention_class, redistribution, evidence_class,
      license_name, agreement_status, agreement_expires_at, valid_from
    ) VALUES (
      'nansen-expired', 'mainnet', 1, 'attribution-enrichment', 'third-party-provisional',
      'nansen', 'provider', 'raw-hot-local', 'internal-only', 'public-market-data',
      'nansen-api-tos', 'active', TIMESTAMPTZ '2026-01-15 00:00:00+00',
      TIMESTAMPTZ '2026-01-01 00:00:00+00'
    );
    INSERT INTO source_capability_binding (
      source_id, network, source_version, capability_id
    ) VALUES (
      'primary-node', 'mainnet', 2, 'node.replica_cmds'
    );
    INSERT INTO source_endpoint_version (
      source_id, network, source_version, transport, endpoint_version
    ) VALUES (
      'primary-node', 'mainnet', 2, 'node', 'hyperliquid-node-v2'
    );
    INSERT INTO source_health_policy (
      source_id, network, source_version, probe_interval_millis, consecutive_failure_threshold
    ) VALUES (
      'primary-node', 'mainnet', 2, 5000, 3
    );
    INSERT INTO source_probe_result (
      source_id, network, probed_at, probe_sequence, outcome, reason_code, latency_millis
    ) VALUES (
      'primary-node', 'mainnet', TIMESTAMPTZ '2026-02-01 00:00:01+00', 0, 'ok', NULL, 12
    );
    COMMIT;
  " >/dev/null

history_versions="$(
  docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
    psql -X --no-psqlrc --tuples-only --no-align \
    --username alpha --dbname alpha \
    --command \
    "SELECT version::text FROM source_registry WHERE source_id = 'primary-node' AND network = 'mainnet' ORDER BY version;"
)"
if [[ "$history_versions" != $'1\n2' ]]; then
  printf 'postgres-migration-smoke:error source catalog history was not preserved\n%s\n' \
    "$history_versions" >&2
  exit 1
fi

if docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
  psql -X --no-psqlrc --set ON_ERROR_STOP=1 --username alpha --dbname alpha \
  --command "
    INSERT INTO source_registry (
      source_id, network, version, role, trust, operator, operator_kind,
      retention_class, redistribution, evidence_class, valid_from
    ) VALUES (
      'primary-node', 'mainnet', 3, 'committed-primary', 'locally-verified-committed',
      'alpha-desk', 'local-node', 'raw-indefinite', 'private-operator-evidence',
      'committed-block', TIMESTAMPTZ '2026-03-01 00:00:00+00'
    );
  " >/dev/null 2>&1; then
  printf 'postgres-migration-smoke:error a second current source row was accepted\n' >&2
  exit 1
fi

scheduled_sources="$(
  docker exec --env PGPASSWORD=alpha_dev_only "$CONTAINER_NAME" \
    psql -X --no-psqlrc --tuples-only --no-align \
    --username alpha --dbname alpha \
    --command \
    "SELECT source_id FROM source_registry WHERE valid_to IS NULL AND (license_name IS NULL OR (agreement_status = 'active' AND (agreement_expires_at IS NULL OR agreement_expires_at > TIMESTAMPTZ '2026-02-01 00:00:00+00'))) ORDER BY source_id, network;"
)"
if [[ "$scheduled_sources" != $'nansen-active\nprimary-node\nprimary-node' ]]; then
  printf 'postgres-migration-smoke:error disabled or expired provider agreements were scheduled\n%s\n' \
    "$scheduled_sources" >&2
  exit 1
fi

printf 'postgres-migration-smoke:ok image=%s\n' "$POSTGRES_IMAGE"
