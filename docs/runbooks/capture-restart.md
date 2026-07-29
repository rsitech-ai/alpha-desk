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
one per-height raw observation through the real node adapter and spool, sends
SIGTERM, restarts the same binary against the same spool, archive, publication
journal, stream, and chain, then drip-feeds the remaining blocks. It does not
qualify action-bearing Hyperliquid semantics, a live source, crash failpoints,
host reboot, or a multi-node production deployment.

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
- three byte-identical raw observations in both spool and Parquet, three
  archived empty blocks, and three acknowledged block publications;
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

## Failed restart or shutdown

1. Preserve the complete run directory, especially `report.json`,
   `capture-status.json`, `service.stdout`, `service.stderr`, `nats.log`,
   `postgres.log`, and `archive/`.
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
