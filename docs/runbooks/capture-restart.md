# Capture restart and retained evidence

## Current qualification boundary

The qualified restart lane is deterministic and uses synthetic node-format
records:

```sh
just capture-e2e
just capture-outage-e2e
just capture-soak 10m
```

`capture-e2e` owns fresh PostgreSQL and authenticated NATS containers, writes
one per-height committed observation and one auxiliary fill observation
through the real node adapters and spools, sends SIGTERM, restarts the same
binary against the same spools, archive, publication journal, stream, and
chain, then drip-feeds the remaining records. It does not qualify
action-bearing Hyperliquid semantics, a live source, crash failpoints, host
reboot, or a multi-node production deployment.

`capture-outage-e2e` pauses only its disposable NATS and PostgreSQL containers.
It proves that local source acquisition continues into the fsynced spool while
canonical drain is degraded, then requires contiguous catch-up after each
dependency returns. Retained `status-nats-outage.json` and
`status-postgres-outage.json` snapshots must be non-ready with a stable reason
code. This is reconnect/replay evidence, not production-cluster failover
qualification.

`hl-capture run` is the tested command. The committed mapper deliberately
rejects non-empty action bundles until a qualified corpus and response mapping
exist. Do not treat empty synthetic records as live qualification or change the
report's `live_source_qualified` field.

## Verify a retained run

Use the report path printed by the command:

```sh
jq . target/evidence/capture-e2e/<run-id>/report.json
cargo +1.97.1 run -p archive-inspect --locked --offline -- \
  verify target/evidence/capture-e2e/<run-id>/archive
cargo +1.97.1 run -p spool-inspect --locked --offline -- \
  verify target/evidence/capture-e2e/<run-id>/spool/synthetic-fixture
cargo +1.97.1 run -p hl-capture --locked --offline -- \
  status \
  --config target/evidence/capture-e2e/<run-id>/capture.toml \
  --json
```

A successful default run has:

- `mode` equal to `synthetic-node-source`;
- `live_source_qualified` equal to `false`;
- `restart_count` equal to `1`;
- `clean_shutdown` equal to `true`;
- three committed observations retained in the committed hot spool, three
  auxiliary observations retained in Parquet with an archive-verified pruned
  hot spool, three archived empty blocks, and three acknowledged block
  publications;
- `auxiliary_local_sequence` and `auxiliary_spool_records` equal to three,
  while `auxiliary_spool_summary` reports zero hot records;
- a terminal status with `ready=false`, `health=yellow`, no pending blocks,
  and the expected durable height.

A successful outage run additionally has:

- `mode` equal to `synthetic-node-source-dependency-outage`;
- `outage_mode` equal to `nats-postgres`;
- `nats_outage_spool_records` equal to `3`;
- `postgres_outage_spool_records` equal to the five-block test count;
- final raw, spool, canonical block, and acknowledgement counts all equal to
  the configured block count.

The retained configuration contains credential paths but no credential values.
Its temporary credential directory is removed during cleanup, so it cannot be
used to reconnect later.

## Disk reserve stop

`capture_disk.insufficient_space` means the runtime could not preserve
`runtime.disk_reserve_bytes` plus conservative headroom for the next spool or
archive write. The source observation is not acknowledged at that boundary.

1. Preserve the status, spool, archive, and filesystem-capacity evidence.
2. Stop only the affected capture service; do not delete spool or archive
   objects to manufacture headroom.
3. Expand the approved filesystem or move the complete verified dataset under
   a documented migration procedure.
4. Verify spool and archive manifests before restart.
5. Confirm the next expected canonical height and raw cursor before allowing
   the source to resume.

The absolute reserve is an enforced write boundary. The design's percentage
GREEN/AMBER/RED health policy and alerts still need their own runtime metric and
suppression integration.

## Auxiliary Node V1 sources

Each enabled `node-line` source has an entry in the V4 `auxiliary_sources`
array. Before calling an auxiliary source healthy, require all of the
following:

- `health` is `healthy`;
- `unarchived_records` is zero after the bounded flush interval;
- `partial_line` is false or is explained by an actively growing writer;
- `local_sequence`, `durable_offset`, and `last_durable_wall_micros` are
  present; and
- the source remains `unqualified` unless an approved production
  qualification manifest has been activated.

`quarantined` means the exact offending bytes are durable and replayable; it
does not mean the record was accepted semantically. Preserve the spool,
archive, status snapshot, source build identity, and reason code. Do not edit
the line, reset the cursor, or relabel the source as qualified. `latched` means
the task failed closed and requires operator diagnosis before restart.
Read `quarantine_reason` for the retained parser/schema cause and
`last_error_reason` for a concurrent retry or terminal failure; an outage must
not overwrite the quarantine cause.

A missing source file at startup is retryable and appears as
`source.temporary_disconnect`. A cursor regression is not retryable because a
replacement/truncated file may have made the unseen byte range ambiguous.

### Auxiliary archive checkpoint recovery authority

Each auxiliary source spool directory contains
`auxiliary-archive-checkpoint-v1.json` after its first verified archive commit.
This file is a recovery authority: it binds the source contract and durable
cursor/sequence to the exact archive path identity, raw manifest receipts,
spool segment hashes, receive time, and retained quarantine history. The
runtime may prune an archive-verified hot spool segment only after publishing
this checkpoint durably.

- Preserve and back up the checkpoint together with its complete source spool
  directory and archive. A spool-only or archive-only copy is not a restorable
  capture state.
- Restore to the same canonical filesystem archive path and preserve the
  verified archive contents. Wiping and recreating an archive at the same path
  fails closed because its identity and receipts no longer match the
  checkpoint.
- An unpublished temporary checkpoint file is discarded during recovery; a
  malformed or unverifiable published checkpoint is a RED corruption signal.
- Never edit or delete the checkpoint, cursor, spool manifests, or raw archive
  objects to force progress. Preserve the evidence and diagnose the exact
  reason code.
- V4 `auxiliary_sources[].spool_records` is the cumulative fsynced record count
  carried across verified pruning. `spool-inspect` reports only records still
  retained in the hot spool, so zero hot records with a positive cumulative
  status count is expected after a successful archive checkpoint.

## Failed restart or shutdown

1. Preserve the complete run directory, especially `report.json`,
   `capture-status.json`, `service.stdout`, `service.stderr`, `nats.log`,
   `postgres.log`, `archive/`, every source spool directory, and every
   `auxiliary-archive-checkpoint-v1.json`.
2. Record the exact stable reason code and the last verified durable height.
3. Verify the archive without modifying it. A failed verification is a RED
   corruption signal.
4. Do not delete or edit archive objects, PostgreSQL journal rows, stream
   state, spool segments, or cursors to force progress.
5. Reproduce with the smallest self-contained `just capture-e2e` run. If the
   failure is deterministic, retain both evidence directories.

The E2E cleanup removes only resources bearing its unique run-owned names. It
does not stop the host PostgreSQL service, shared development NATS, or unrelated
containers and volumes.
