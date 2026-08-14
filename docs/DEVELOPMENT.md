# Development Guide

## Toolchains

- Rust/Cargo: `1.97.1`, pinned by `rust-toolchain.toml`
- Rust edition: 2024
- Swift: 6.3
- Docker Compose: required only for dependency-stack and integration smokes
- Parser fuzzing: `nightly-2026-07-16` plus `cargo-fuzz 0.13.2`

Use the committed lockfiles. Normal verification is offline after dependencies have been fetched.

## Start here

```sh
just --list
just verify
just generated
just spool-verify
SOURCE_DATE_EPOCH=1784894400 just reproducible
```

The reproducibility check intentionally requires an explicit unsigned
`SOURCE_DATE_EPOCH`; it does not infer a timestamp from the working tree or
ambient clock.

Focused Rust work should use the smallest package:

```sh
cargo +1.97.1 test -p <package> --locked --offline
cargo +1.97.1 clippy -p <package> --all-targets --locked --offline -- -D warnings
```

The full local dependency-stack smoke is:

```sh
just stage-0-compose-smoke
```

`just dev-up` starts PostgreSQL, NATS, ClickHouse, MinIO, and related development dependencies. It does not start Alpha Desk services or a UI.

The focused durable-spool checks are:

```sh
cargo +1.97.1 test -p hl-capture --test spool_recovery --locked --offline
cargo +1.97.1 test -p spool-inspect --locked --offline
just spool-verify
just spool-fuzz
```

`just spool-fuzz` runs for 60 seconds by default. It validates parser safety, not service uptime or
source completeness. The normative framing and recovery contract is
[`formats/spool-v1.md`](formats/spool-v1.md).

The focused primary-node boundary checks are:

```sh
cargo +1.97.1 test -p hl-protocol --test node_golden --locked --offline
cargo +1.97.1 test -p hl-capture --test node_adapter --locked --offline
```

The checked node corpus is made from normalized official documentation
examples, not operator recordings. Read the exact cursor, quarantine, and
qualification boundary in
[`adapters/hyperliquid-node.md`](adapters/hyperliquid-node.md).
Committed node-directory configurations must declare
`replica_cmds_style = "actions-and-responses"`; `hl-capture check-config`
rejects action-only or rolling-history node profiles.

The deterministic public trade mapping and semantic-version boundary checks
are:

```sh
cargo +1.97.1 test -p canonical-events --test node_mapping --locked --offline
cargo +1.97.1 test -p canonical-events --test upcast --locked --offline
cargo +1.97.1 test -p canonical-inspect --locked --offline
cargo +1.97.1 run -p canonical-inspect --locked --offline -- \
  canonicalize --root . \
  --manifest fixtures/canonical/node-v1/inspect.toml \
  --output target/canonical-node-v1.json
```

The output path must not exist. The inspector publishes one atomically written
manifest containing source hashes, mapping disposition, event/payload hashes,
and the canonical block hash. The checked result remains normalized public
documentation evidence with provisional confirmation; it is not real-node or
production qualification.

The focused source-trust boundary checks are:

```sh
cargo +1.97.1 test -p hl-protocol --test source_trust --locked --offline
cargo +1.97.1 test -p hl-capture --test config --locked --offline
```

Every configured source must declare a trust class compatible with its
observation class. Watermark eligibility and publication lane are derived from
that validated pair. See
[`adapters/source-priority.md`](adapters/source-priority.md).

The focused canonical continuity checks are:

```sh
cargo +1.97.1 test -p hl-capture --test sequencer --locked --offline
cargo +1.97.1 clippy -p hl-capture --all-targets --all-features \
  --locked --offline -- -D warnings
```

The synchronous state machine has bounded pending/recent histories, isolates
provisional watermarks, emits explicit recovery decisions for evicted history,
and permanently latches red on source divergence. It does not persist a
durable cursor: archive-before-cursor integration remains part of the archive
and long-running runtime milestones. Operational response is documented in
[`runbooks/committed-gap.md`](runbooks/committed-gap.md) and
[`runbooks/source-divergence.md`](runbooks/source-divergence.md).

The capture incident, archive/publication journal, and cursor migrations are
checked against the exact pinned PostgreSQL image in an isolated no-port
container:

```sh
just postgres-migration-smoke
```

Focused progress-store checks are:

```sh
cargo +1.97.1 test -p hl-capture --test progress_store --locked --offline
ALPHA_DESK_POSTGRES_TEST_URL='<disposable migrated database URL>' \
  cargo +1.97.1 test -p hl-capture --test postgres_progress \
  --locked --offline -- --nocapture
```

The PostgreSQL test intentionally skips unless its explicit environment
variable is set; a default green result is not integration evidence. Use only
a disposable migrated database because the selected test creates and removes
its uniquely named rows. The journal and recovery rules are frozen in
[`contracts/capture-progress-v1.md`](contracts/capture-progress-v1.md).

The focused immutable-archive checks are:

```sh
cargo +1.97.1 test -p hl-analytics --test archive --locked --offline
cargo +1.97.1 test -p archive-inspect --locked --offline
cargo +1.97.1 clippy -p storage-ports -p canonical-archive -p hl-analytics -p archive-inspect \
  --all-targets --all-features --locked --offline -- -D warnings
cargo +1.97.1 run -p archive-inspect --locked --offline -- verify <archive-root>
cargo +1.97.1 run -p archive-inspect --locked --offline -- count <archive-root>
just archive-verify
just archive-count
```

The `canonical-archive` foundation preserves exact canonical Protobuf
envelopes and raw source bytes,
verifies complete manifest chains and all requested objects before yielding,
and supports idempotent immutable compaction without deleting prior
generations. `count` uses DataFusion as an independent Parquet readability and
row-count check. The normative format and recovery boundary is
[`formats/archive-manifest-v1.md`](formats/archive-manifest-v1.md). This is
storage-layer evidence only. Capture now coordinates the archive, PostgreSQL
publication journal, JetStream acknowledgement, and contiguous cursor in that
order.
The default `just` commands use the byte-reproducible synthetic fixture under
`fixtures/archive/valid-v1`; empty archive roots fail closed.

## Capture runtime evidence

Validate the checked-in non-secret configuration and inspect an existing
atomic status snapshot with:

```sh
cargo +1.97.1 run -p hl-capture --locked --offline -- \
  check-config --config config/capture.example.toml
cargo +1.97.1 run -p hl-capture --locked --offline -- \
  status --config <retained-capture-config> --json
```

The V5 status contract is what `hl-capture run` writes. It keeps every V4
field, requires a fail-closed `maintenance` object, and omits last-heartbeat
rates until a window is sampled. Readers still accept inactive
`hl.capture.status.v4` (no `maintenance`). See
[`contracts/capture-status-v5.md`](contracts/capture-status-v5.md) and
[`contracts/capture-status-v4.md`](contracts/capture-status-v4.md).

When `runtime.status_listen` is a loopback address, `hl-capture run` also
serves that snapshot over HTTP (`GET /status`, `GET /healthz`, SSE
`GET /events`). `hl-capture serve-status --config <path> [--listen <addr>]`
serves the same file without starting capture. Bind addresses must be
loopback. `GET /status` fail-closed-reads inactive `hl.capture.status.v4`
(no `maintenance`) and `hl.capture.status.v5` (`maintenance` required), and
returns the snapshot bytes as read. This HTTP surface does not replace
`hl-api` `/v1/capture/status`, which reads the status file on disk. Writer
schema V5 is not Stage 1 PASS or live-source qualification.

The self-contained runtime E2E creates fresh test-owned PostgreSQL 18.4 and
authenticated NATS 2.14.3 containers on Docker-assigned loopback ports. It
drip-feeds deterministic empty transaction-block records and auxiliary fill
records through the real node-directory and NodeLine adapters in
`hl-capture run`, verifies raw spool/checkpoint durability, archives
byte-identical raw observations plus committed blocks, performs one clean
process restart against the same spool/archive/journal state, verifies V4
auxiliary cursor recovery plus PostgreSQL and JetStream acknowledgements, and
proves a final bounded SIGTERM shutdown:

```sh
just capture-e2e
just capture-outage-e2e
just capture-failover-e2e
just capture-soak 10m
```

Each run retains an atomic report and non-secret diagnostic artifacts under
`target/evidence/capture-e2e/<run-id>/`. The report records the binary hash,
dependency versions, block/publication counts, restart count, runtime,
resource high-water marks, spool/archive summaries, log byte counts, status
schema, auxiliary cumulative sequence and pruned hot-spool summary, outage
backlog samples, final capture backlog, final disk-free basis points, and
shutdown result. The test removes only its disposable containers, network, and
temporary secret directory. Treat each source spool's
`auxiliary-archive-checkpoint-v1.json` as recovery authority and preserve it
with the complete spool and archive; see
[`runbooks/capture-restart.md`](runbooks/capture-restart.md).

`capture-outage-e2e` uses five records and pauses its test-owned NATS and
PostgreSQL containers in turn. It requires the spool to grow during each
outage, independently requires the auxiliary NodeLine source to remain healthy
and archive every corresponding fill with zero unarchived records, captures
the degraded atomic status, restores the dependency, and requires a positive
visible committed capture backlog during both outages and exact
raw/spool/block/publication parity with zero final backlog at the contiguous
cursor. The test never stops or modifies host PostgreSQL, shared NATS, or
unrelated containers. `runtime.postgres_operation_timeout_millis`
independently bounds PostgreSQL connection and progress operations; it is not
coupled to the JetStream publication timeout.

`capture-failover-e2e` uses two synthetic node directories. It withholds the
second primary height while making a later primary height visible, supplies a
complete independent range, verifies the exact create-once failover record and
yellow-ready Status V4 state, performs a clean restart, repairs the primary,
and proves the runtime still drains from the independent source. It requires
two five-record committed spools, one archive-checkpointed auxiliary stream,
fifteen raw observations, five canonical blocks and
publications, zero final active backlog, and no automatic failback.

This is a synthetic node-format runtime-mechanics lane. Its report deliberately
uses `"mode": "synthetic-node-source"` for restart/soak and
`"mode": "synthetic-node-source-dependency-outage"` for the fault lane.
Failover evidence uses `"mode": "synthetic-dual-source-failover"`. Every
report contains
`"live_source_qualified": false`. The production `run` command is connected,
but the committed mapper accepts only structurally valid blocks with no action
bundles. Action-bearing records fail closed with
`canonical_mapping.unsupported_committed_actions`. Closed, verified spool
segments are archived idempotently into raw Parquet before restart completion
or graceful task exit. Spool verification and replay stream one bounded record
at a time; raw Parquet batches are additionally bounded by record count and
uncompressed payload bytes. Segment targets above 512 MiB are rejected pending
production benchmarking. One-node loopback password authentication and tmpfs
JetStream also do not qualify production TLS, identity, or three-replica
durability.
See [`runbooks/capture-restart.md`](runbooks/capture-restart.md) for retained
evidence and restart diagnosis.

## Engineering rules

- Write a focused failing test before behavior changes.
- Keep deterministic domain logic synchronous and separate from adapters.
- Validate all boundary input and reject unknown configuration keys.
- Preserve source bytes and explicit version/provenance fields.
- Make retry, quarantine, stop, and recovery behavior typed and observable.
- Use bounded queues, payloads, logs, timeouts, and shutdown paths.
- Do not add execution, signer, credential, or order-placement capability to V1.
- Do not weaken a gate to make a change pass.

## Stage plans

The detailed implementation plans live under `docs/superpowers/plans/`. They are approved design inputs and retain their original checklist state. Current implementation evidence is recorded in `docs/STATUS.md`.

Stage 1 normally requires a verified signed `stage-0-foundations` tag. Work developed before that external gate closes must remain clearly labeled as unreleased development and cannot be used to claim the gate passed.

## Test boundaries

- Unit tests prove pure invariants and error semantics.
- Boundary integration tests prove serialization, storage, process, and dependency contracts.
- Runtime smokes prove real startup, readiness, shutdown, listener release, and owned-resource cleanup.
- Synthetic long-running evidence proves runtime mechanics only; live-source
  qualification additionally requires the approved source authority, mapper,
  corpus, and recovery gates.

## Sensitive material

Never commit credentials, private keys, real wallet labels, proprietary operator-feed fixtures, private alpha thresholds/results, model bundles, production inventory, certificates, or internal hostnames. Use synthetic or explicitly redistributable fixtures. Report suspected exposures through [SECURITY.md](../SECURITY.md), not a public issue.
