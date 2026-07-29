# Capture restart and retained evidence

## Current qualification boundary

The qualified restart lane is deterministic and synthetic:

```sh
just capture-e2e
just capture-soak 10m
```

`capture-e2e` owns fresh PostgreSQL and authenticated NATS containers, writes
one block, sends SIGTERM, restarts the same binary against the same archive,
publication journal, stream, and chain, then writes the remaining blocks. It
does not qualify a live Hyperliquid source, crash failpoints, host reboot, or a
multi-node production deployment.

The production `hl-capture run` command remains fail-closed until the committed
node-block mapper is implemented. Do not substitute `fixture-replay` for a live
service or change the report's `live_source_qualified` field.

## Verify a retained run

Use the report path printed by the command:

```sh
jq . target/evidence/capture-e2e/<run-id>/report.json
cargo +1.97.1 run -p archive-inspect --locked --offline -- \
  verify target/evidence/capture-e2e/<run-id>/archive
cargo +1.97.1 run -p hl-capture --locked --offline -- \
  status \
  --config target/evidence/capture-e2e/<run-id>/capture.toml \
  --json
```

A successful default run has:

- `mode` equal to `synthetic-fixture`;
- `live_source_qualified` equal to `false`;
- `restart_count` equal to `1`;
- `clean_shutdown` equal to `true`;
- three archived blocks and six acknowledged publications;
- a terminal status with `ready=false`, `health=yellow`, no pending blocks,
  and the expected durable height.

The retained configuration contains credential paths but no credential values.
Its temporary credential directory is removed during cleanup, so it cannot be
used to reconnect later.

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
