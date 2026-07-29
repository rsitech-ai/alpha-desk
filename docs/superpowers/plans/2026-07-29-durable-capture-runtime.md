# Durable Capture Runtime and JetStream Publication

## Goal

- User-visible outcome: Alpha Desk has a read-only `hl-capture` process that can
  run continuously against approved local source adapters, archive every
  committed canonical block before publication, recover deterministically
  after a crash, publish idempotent canonical messages through NATS JetStream,
  expose safe status/health, and stop without detached work.
- How to see it working:
  - `just dev-up` starts the loopback-only dependency stack and freezes the
    reviewed JetStream policies;
  - `hl-capture run --config <path>` remains running, becomes ready only after
    archive/progress/bus recovery completes, and exits cleanly on SIGINT or
    SIGTERM;
  - `hl-capture status --config <path> --json` reports durable block progress,
    pending publication, source cursors, gaps/divergence, archive identity,
    bus acknowledgement, queue pressure, free space, build identity, and health
    without source payloads or credentials;
  - a restart test and bounded soak prove archive-before-cursor ordering, safe
    republish after acknowledgement loss, one retained JetStream message per
    event ID, contiguous progress, and no data loss.

## Progress

- [x] (2026-07-29) Froze the 17-subject publication contract and exhaustive
  canonical-event routing.
- [x] (2026-07-29) Implemented the pinned JetStream publisher, bounded
  acknowledgement path, exact stream bootstrap policy, and a real one-node
  deduplication smoke.
- [x] (2026-07-29) Added generated owner-only local credentials, separate
  bootstrap/capture/reader identities, subject-level ACLs, anonymous denial,
  secret-file validation, and a live permission probe.
- [x] (2026-07-29) Added backend-neutral hash-bound progress types, the
  deterministic in-memory reference store, and transition tests.
- [x] (2026-07-29) Added the PostgreSQL V2 archive/publication journal,
  serializable adapter, full-`u64` encoding, migration constraints, and a real
  disconnect/reconnect integration test.
- [x] (2026-07-29) Implemented archive-before-publish coordination, exact
  crash-boundary recovery, archive-only prefix reconciliation, and source-free
  journal recovery.
- [x] (2026-07-29) Implemented owned lifecycle, strict CLI/configuration,
  atomic status, real dependency wiring, explicit synthetic fixture replay,
  and signal-safe bounded shutdown.
- [x] (2026-07-29) Added a self-contained PostgreSQL/NATS process E2E with one
  durable restart and an atomic bounded-soak evidence report.
- [ ] Wire the committed primary-node mapper/source spool into `run`, add
  loopback health/metrics, and complete the crash-failpoint restart matrix.

## Decisions and discoveries

- JetStream acknowledgement and PostgreSQL progress are independently
  retryable; the journal records one immutable plan and one exact receipt per
  publication instead of treating either system as a transaction participant.
- The legacy cursor table does not contain publication identities. Migration
  V2 therefore fails closed when legacy cursor rows exist instead of inventing
  acknowledgements or silently trusting unknown progress.
- PostgreSQL `bigint` cannot represent the Rust `u64` domain. Numeric values
  are constrained to `u64::MAX` and encoded through checked decimal text.
- A host PostgreSQL process already owns loopback port 5432 on the development
  machine. Real adapter evidence used a disposable pinned container on a
  Docker-assigned loopback port; no host service was stopped or modified.
- Compose one-shot initializer containers are expected to exit successfully.
  `dev-up` now uses its own bounded readiness checker instead of treating an
  exited successful initializer as a failed `compose up --wait` service.
- A diagnostic command briefly printed the first generated development NATS
  passwords. Those files were moved to the system Trash, a new credential set
  was generated, and the NATS container was recreated before further evidence.
  No committed, user, or external credentials were involved.
- The coordinator is verified at every durable boundary: archive, journal,
  publication acknowledgement, and contiguous cursor. Startup can reconstruct
  an archive-only contiguous prefix without rereading the source and can
  complete a journalled block without source replay.
- The PostgreSQL adapter was requalified against the current V2 schema on a
  disposable pinned PostgreSQL 18.4 instance after the archive object and
  schema identity fields were added.
- The runtime E2E owns fresh pinned PostgreSQL and authenticated NATS
  containers on random loopback ports. It no longer depends on an ambient
  development stack, preserves non-secret evidence, and removes only
  test-owned infrastructure and temporary secret files.
- `fixture-replay` is the only enabled ingestion command. Production `run`
  fails closed with `capture_runtime.committed_source_mapper_unavailable`
  until the real node block-to-canonical mapper and spool/cursor loop exist.
- A graceful process restart now proves coordinator recovery against the same
  archive, PostgreSQL journal/cursor, and JetStream stream. Crash failpoints,
  forced termination, and live-source restart qualification remain open.

## Current State

- Relevant paths:
  - `services/hl-capture/` contains validated capture configuration, primary
    node file adapters, a crash-safe per-source spool, quarantine records, and
    a deterministic in-memory canonical sequencer.
  - `crates/canonical-archive/` contains the verified local canonical and raw
    Parquet archive implementation behind `storage-ports` contracts.
  - `schemas/postgres/0001_capture_incidents.sql` contains incident and final
    sequencer-cursor tables, but no archived/publish-pending block journal.
  - `infra/docker-compose/` already pins NATS Server `2.14.3`, runs JetStream on
    file storage, and initializes development streams.
  - `crates/telemetry/` provides structured logging/provenance and health
    foundations but no capture-runtime metrics.
- Existing behavior:
  - adapters acknowledge progress only when a caller supplies a durable
    receipt; speculative reads are replayed after restart;
  - sequencer `Commit` is explicitly logical and must be archived before a
    durable cursor advances;
  - archive append is immutable and idempotent, and full verification happens
    before replay yields any block;
  - `hl-capture fixture-replay` runs the real archive, PostgreSQL progress
    store, JetStream publisher, recovery coordinator, owned lifecycle, atomic
    status, restart lane, and bounded soak harness;
  - `hl-capture run` is deliberately unavailable until a committed source
    mapper can preserve the approved source authority and spool semantics.
- Constraints:
  - implement approved Truth Layer Tasks 8 and 9 and design sections 10.8,
    24, and 26; do not invent a second transport or treat NATS as the archive;
  - the app remains read-only and contains no private-key, signing, order, or
    trade-execution path;
  - canonical archive durability precedes PostgreSQL progress, and durable
    contiguous progress advances only after every canonical publication has a
    JetStream acknowledgement;
  - all writes are idempotent and hash-bound because JetStream and PostgreSQL
    are independently retryable systems with no distributed transaction;
  - NATS failure is degraded operation: raw evidence continues to spool within
    configured disk/queue limits, committed blocks remain safely republishable,
    and readiness/health reflect the backlog;
  - bounded channels, timeouts, reconnect backoff, payload limits, disk limits,
    cancellation, and task joining are mandatory;
  - credentials are path references or environment-injected values and never
    appear in configuration serialization, status, logs, fixtures, or Git.

## Target State

- Desired behavior:
  - `bus` defines backend-neutral, validated publication requests and receipts,
    an exact subject mapper, a `CanonicalPublisher` port, and a concrete
    `async-nats` JetStream publisher.
  - Every committed event uses its lowercase `EventId` as `Nats-Msg-Id`, binds
    the expected stream, carries canonical schema/content/block/archive hashes,
    waits for the server publish acknowledgement, and rejects a duplicate ID
    with divergent bytes before transport.
  - Development stream definitions use file storage, limits retention, a
    six-hour maximum age, bounded bytes/message size, an explicit duplicate
    window, and one replica. The production policy documents and validates
    three replicas and a configurable six-to-twenty-four-hour measured window.
  - PostgreSQL owns a journal row per chain/block. States are
    `archived_pending`, `publishing`, `acknowledged`, or `quarantined`; every
    transition validates the canonical block hash, archive manifest hash,
    receipt ID, event count, and publication rolling hash.
  - The coordinator performs:
    `archive -> record archived -> publish block/events -> record ack ->
    advance contiguous cursor -> acknowledge source cursor`. Each operation is
    retryable without changing output identity.
  - Startup verifies configuration/build/schema, recovers and verifies spool
    tails/manifests, verifies the archive, loads journal/cursor state,
    reconciles archive-only blocks, republishes unacknowledged blocks, then
    opens adapters and readiness.
  - `CaptureApp` owns every task under one cancellation token and bounded
    channels. First fatal error or panic cancels peers, drains only already
    durable work within a bounded grace period, joins all tasks, and exits
    non-zero.
  - Status is read-only, bounded, stable JSON with an explicit schema version
    and atomic snapshot time. Health becomes yellow for recoverable lag/bus
    outage and red for corruption, divergence, cursor regression, exhausted
    disk reserve, or unknown progress origin.
  - A deterministic fixture runtime and fault points exercise crash boundaries
    without weakening the production binary or adding test-only silent
    fallbacks.
- Non-goals:
  - no independent/public/historical source adapter is fabricated in this
    milestone; missing real source authority remains an explicit Stage 1 gap;
  - no state reducer, ClickHouse load, research feature, API product surface,
    SwiftUI application, or trading behavior;
  - no multi-host NATS cluster qualification, external TLS/identity issuance,
    signed gate, public release, or remote deployment;
  - no claim that a one-node development JetStream acknowledgement survives
    host power loss; production requires the documented three-replica quorum.

## Risks and Failure Modes

- A crash between archive, journal, publish, acknowledgement, and cursor writes
  can lose progress or duplicate effects unless each boundary is independently
  discoverable and idempotent.
- JetStream deduplication tracks only `Nats-Msg-Id` within a configured window
  and does not compare payloads; application-side ID-to-content binding is
  therefore required.
- Advancing a source or sequencer cursor before every archive/publication
  receipt is durable can make accepted source bytes unrecoverable.
- PostgreSQL numeric conversion can narrow `u64` block/cursor values or accept
  negative/out-of-range values unless encoded and checked explicitly.
- Reconnect loops, producer pressure, or an unavailable NATS server can grow
  memory or disk without bound.
- A task panic, dropped join handle, signal race, or unbounded shutdown drain
  can leave the process apparently stopped while work is still active.
- Status or structured errors can leak source payloads, credential paths,
  connection strings, or secret-bearing transport errors.
- Development stream bootstrap can accidentally weaken production policy or
  allow capture credentials to publish derived state/feature/signal subjects.
- Pulling the full default `async-nats` feature set can add avoidable crypto,
  websocket, KV, and object-store supply-chain surface.
- Local integration tests can appear green while no real NATS/PostgreSQL
  process ran; tests must fail closed when the declared integration lane is
  selected and record exact server versions.

## Milestones

### M1. Freeze publication contracts, subjects, and red tests

- Goal: make message identity, archive binding, subject routing, receipt
  validation, and duplicate-divergence semantics explicit before network I/O.
- Files / systems:
  - `services/hl-capture/src/bus/mod.rs`
  - `services/hl-capture/src/bus/subjects.rs`
  - `services/hl-capture/tests/publication_contract.rs`
  - `docs/contracts/nats-subjects-v1.md`
- Changes:
  - define exact subject/stream enums and exhaustive event-kind routing;
  - define deterministic committed block/event publication records and
    validated acknowledgement receipts;
  - require a matching `ArchiveReceipt` and reject block height/hash, schema,
    event identity, payload-hash, or subject mismatches;
  - add a deterministic in-memory publisher only as a test implementation of
    the production port, including acknowledgement-loss and conflicting-ID
    cases.
- Verification:
  - `cargo +1.97.1 test -p hl-capture --test publication_contract --locked`
- Expected result: tests first fail because the contracts are absent, then pass
  with no network and prove the fail-closed boundary.

### M2. Implement and qualify the JetStream publisher

- Goal: turn validated archived publications into acknowledged, idempotent
  JetStream messages under frozen stream policy.
- Files / systems:
  - `services/hl-capture/src/bus/jetstream.rs`
  - `services/hl-capture/tests/jetstream.rs`
  - `infra/docker-compose/nats/init-streams.sh`
  - `infra/docker-compose/nats/bootstrap.json`
  - `infra/docker-compose/nats/nats.conf`
  - `infra/monitoring/alerts/nats.yml`
  - workspace/package dependency and supply-chain policy files
- Changes:
  - pin `async-nats 0.50.0` with only the JetStream/server compatibility and
    chosen TLS/auth features;
  - set `Nats-Msg-Id`, `Nats-Expected-Stream`, application schema/hash headers,
    publish through JetStream, await both send and server acknowledgement, and
    retain sequence/duplicate metadata in the typed receipt;
  - freeze file/limits/age/size/replica/duplicate-window/max-message policies;
  - create durable pull-consumer and dead-letter policy contracts;
  - validate capture and read-only permission boundaries in bootstrap tests.
- Verification:
  - focused unit tests with a publisher seam;
  - `just dev-up`;
  - `cargo +1.97.1 test -p hl-capture --test jetstream --locked`;
  - `docker compose ... exec nats-init nats stream report`;
  - `just dev-down`.
- Expected result: initial, duplicate, and simulated lost-ack republish resolve
  to one retained event; divergent bytes for the same event ID fail closed.

### M3. Add the PostgreSQL archive/publication progress journal

- Goal: make every cross-system boundary durable and restart-discoverable
  without pretending PostgreSQL and JetStream share a transaction.
- Files / systems:
  - `crates/storage-ports/src/capture_progress.rs`
  - `crates/storage-ports/src/lib.rs`
  - `services/hl-capture/src/progress/mod.rs`
  - `services/hl-capture/src/progress/postgres.rs`
  - `services/hl-capture/tests/progress_store.rs`
  - `schemas/postgres/0002_capture_progress.sql`
  - `infra/docker-compose/postgres/init.sql`
- Changes:
  - define typed progress snapshots/transitions and stable error codes;
  - add hash-bound pending/acknowledged publication journal tables and strict
    constraints without narrowing `u64`;
  - use transactions plus compare-and-set cursor versions to prevent writers
    from skipping, regressing, or forking progress;
  - expose bounded pending scans for startup republish and status.
- Verification:
  - migration transaction/constraint smoke on the pinned PostgreSQL image;
  - focused transition/idempotency/conflict/concurrency tests.
- Expected result: every crash boundary is represented by either archive
  state, a pending journal row, or an acknowledged contiguous cursor.

### M4. Implement archive-before-publish coordination and recovery

- Goal: integrate the existing sequencer/archive with the bus and journal in
  the only safe order.
- Files / systems:
  - `services/hl-capture/src/coordinator.rs`
  - `services/hl-capture/tests/coordinator.rs`
  - archive construction boundary used by `hl-capture`
- Changes:
  - consume logical sequencer decisions and perform the durable transition
    pipeline;
  - reconcile archive-only blocks, resume pending publications, and verify
    cursor/hash continuity at startup;
  - add deterministic failpoints after archive, journal, publish, ack, and
    cursor operations;
  - refuse readiness on corruption, unknown origin, fork, or divergence.
- Verification:
  - table-driven crash/restart matrix with identical final archive, cursor,
    publication IDs, hashes, and event counts.
- Expected result: retries and crashes never lose a committed block, skip a
  height, or produce a second logical publication.

### M5. Build the owned long-running process and safe operator surface

- Goal: replace the immediately exiting binary with a service that owns its
  lifecycle and can be diagnosed without reading raw data.
- Files / systems:
  - `services/hl-capture/src/app.rs`
  - `services/hl-capture/src/shutdown.rs`
  - `services/hl-capture/src/status.rs`
  - `services/hl-capture/src/main.rs`
  - `services/hl-capture/src/config.rs`
  - `config/capture.example.toml`
  - `crates/telemetry/src/metrics.rs`
  - `infra/systemd/hl-capture.service.d/override.conf`
  - `infra/monitoring/dashboards/capture.json`
- Changes:
  - add strict `run`, `status`, `check-config`, and fixture-replay CLI commands;
  - validate archive, PostgreSQL, NATS, bind, queue, disk reserve, timeout, and
    shutdown configuration;
  - own all tasks with bounded channels/cancellation/joining and structured
    fatal-error propagation;
  - expose loopback-only readiness/health/metrics and stable JSON status.
- Verification:
  - lifecycle tests for startup failure, panic propagation, SIGINT/SIGTERM,
    bounded drain timeout, no detached tasks, and secret-free output.
- Expected result: the process stays running only when its durable dependencies
  and source authority are valid, and shuts down predictably.
- Current result: the owned process, strict CLI/config, atomic status, startup
  recovery, panic/failure propagation, task joining, and SIGTERM fixture path
  are implemented. Loopback HTTP readiness/health/metrics, disk/backpressure
  enforcement, source task ownership, systemd activation, and dashboard
  evidence remain.

### M6. Add restart E2E, bounded soak, and operator recovery evidence

- Goal: provide the first runnable evidence lane suitable for repeated and
  long-running local tests.
- Files / systems:
  - `services/hl-capture/tests/end_to_end.rs`
  - `tools/ci/capture-soak.sh`
  - `justfile`
  - `docs/runbooks/capture-restart.md`
  - `docs/runbooks/spool-corruption.md`
  - `docs/DEVELOPMENT.md`
- Changes:
  - replay deterministic blocks through the real process and dependencies;
  - kill at each durable failpoint, restart, and verify contiguous progress,
    exact archive hashes, one retained message per event ID, and no data loss;
  - add a bounded configurable soak that writes an atomic JSON evidence report,
    records versions/build/config hashes, resource high-water marks, restart
    count, health transitions, and clean shutdown;
  - document exact diagnosis and recovery without deleting operator evidence.
- Verification:
  - `just capture-e2e`;
  - `just capture-soak 10m` for development smoke;
  - a later retained multi-hour run before Stage 1 qualification.
- Expected result: a user can start the product foundation, accumulate honest
  runtime evidence, stop/restart it, and inspect its durable state.
- Current result: `just capture-e2e` and `just capture-soak <duration>` run
  against fresh test-owned PostgreSQL 18.4 and authenticated NATS 2.14.3,
  retain atomic evidence, restart once, and verify archive/journal/cursor
  agreement. The crash-failpoint matrix, spool corruption injection, and
  retained multi-hour run remain.

### M7. Close the runtime milestone

- Goal: produce reviewable commits and fresh source/runtime/OSS evidence without
  overstating Stage 1 or full-product readiness.
- Files / systems:
  - runtime implementation, schemas, infra, tests, runbooks, status, changelog,
    dependency policy, and this plan.
- Changes:
  - update generated/config/OSS checks and repo status;
  - review dependencies, permission boundaries, secret handling, unsafe policy,
    and intentional files;
  - commit bus and runtime slices separately when each is locally green.
- Verification:
  - strict format and whole-target Clippy;
  - focused unit/integration/restart tests;
  - PostgreSQL migration and JetStream policy smokes;
  - `just verify`, `just generated`, `just oss-audit`;
  - manual process start/status/stop/restart and one bounded soak.
- Expected result: the branch contains a runnable, crash-recoverable truth-layer
  service foundation with reproducible local evidence; remaining Stage 1 and
  full-product gaps stay explicit.

## Verification

- `cargo +1.97.1 fmt --all -- --check`
- `cargo +1.97.1 clippy -p storage-ports -p hl-capture --all-targets --all-features --locked -- -D warnings`
- `cargo +1.97.1 test -p hl-capture --locked`
- `just dev-stack-contract`
- `just capture-e2e`
- `just capture-soak 10m`
- `cargo +1.97.1 deny --locked check`
- `just verify`
- `just generated`
- `just oss-audit`
- `git diff --check`
- Manual smoke:
  1. start the loopback dependency stack and record NATS/PostgreSQL versions;
  2. run `hl-capture` against the committed deterministic fixture;
  3. wait for readiness and inspect secret-free status;
  4. interrupt once during publication and restart from the same state root;
  5. verify archive, PostgreSQL cursor/journal, JetStream stream count and
     message IDs agree;
  6. send SIGTERM and confirm bounded clean shutdown with no remaining process.

## Decision Log

- 2026-07-29: Implement JetStream as the operational bus only. The Parquet
  archive remains authoritative and every publication requires a matching
  archive receipt.
- 2026-07-29: Use `Nats-Msg-Id` for server-side deduplication and retain
  application-side ID-to-content validation. Official NATS documentation says
  duplicate detection compares the ID, not the body, and only within the
  configured duplicate window.
- 2026-07-29: Await the JetStream publish acknowledgement before marking a
  journal row acknowledged. A send future alone does not prove stream storage.
- 2026-07-29: Use a PostgreSQL pending-publication journal plus a contiguous
  cursor rather than a single cursor row. This makes archive-only,
  published-but-unrecorded, and acknowledged progress discoverable after
  crashes.
- 2026-07-29: Keep development at one JetStream replica but encode production
  policy as three replicas. Official NATS documentation notes that a single
  node can acknowledge before an OS-level fsync; a replicated stream
  acknowledges after quorum replication.
- 2026-07-29: Context Hub `0.1.3` had no NATS/async-nats entry. Use official
  NATS documentation, docs.rs for `async-nats 0.50.0`, and the pinned crate
  source as the version-specific fallback. Do not rely on memory for APIs.
- 2026-07-29: Pin `async-nats 0.50.0`; it supports Rust 1.88 and explicit NATS
  Server 2.14 compatibility under this repository's Rust 1.97.1 toolchain.
- 2026-07-29: Do not fabricate the missing independent/historical source
  contracts to make an E2E test look live. Fixture replay proves runtime
  mechanics; real source qualification remains a separate Stage 1 gate.
- 2026-07-29: Accept plaintext NATS only for exact IPv4/IPv6 loopback hosts;
  remote endpoints require `tls://`. Parse the address structurally and reject
  credentials, WebSockets, paths, queries, fragments, port zero, and spoofed
  loopback prefixes.
- 2026-07-29: Keep the E2E dependency stack fully self-contained. A shared
  development NATS process is useful for manual work but cannot establish
  portable restart/soak evidence.
- 2026-07-29: Extract immutable archive mechanics into the reusable
  `canonical-archive` foundation rather than making `hl-capture` depend on the
  `hl-analytics` deployable. Keep the exact Parquet `paste` advisory exception
  fail-closed over only the reviewed archive consumers.

## Progress Log

- 2026-07-29: Canonical/raw archive milestone committed as `9532290` and its
  detached clean-worktree generated/reproducibility gate passed.
- 2026-07-29: Reviewed approved Truth Layer Tasks 8–9, exact subject design,
  existing spool/adapter/sequencer/archive boundaries, PostgreSQL cursor
  migration, NATS 2.14.3 development stack, and official JetStream
  acknowledgement/deduplication/durability semantics.
- 2026-07-29: Current work is M1. Next action is to write publication contract
  tests that fail because the bus boundary and subject mapper do not yet exist.
- 2026-07-29: M1 red/green loop complete: five focused tests now prove
  exhaustive event routing, deterministic archive-bound block/event messages,
  exact canonical event bytes, receipt mismatch rejection, bounded
  ID-to-content tracking, and divergent duplicate rejection. The frozen
  subject/permission/consumer contract is documented. Next action is M2
  JetStream transport and stream-policy qualification.
- 2026-07-29: M1–M4 complete and committed as `e81bf0f`. The focused package,
  disposable PostgreSQL reconnect, NATS policy, dependency, and secret-scan
  gates passed.
- 2026-07-29: M5 runtime slice complete for the explicit fixture lane. Focused
  tests cover strict config/CLI, owner-only secrets, startup recovery,
  lifecycle cancellation/failure/panic joining, atomic status, and the
  deterministic fixture coordinator.
- 2026-07-29: M6 partial runtime evidence is green. Report
  `20260729T122217Z-16653` used fresh PostgreSQL 18.4 and NATS 2.14.3, archived
  three blocks, recorded six acknowledgements, restarted once, emitted zero
  service stdout/stderr bytes, verified the archive, and shut down cleanly.
  Report `20260729T122425Z-18323` ran the restart-enabled 10-second soak with
  ten blocks, twenty acknowledgements, 25.8 MiB peak RSS, zero service output,
  verified archive state, and clean shutdown.
- 2026-07-29: Next action is to pass the full focused local gate, commit M5/M6,
  then implement the committed node mapper and source-spool-to-coordinator
  loop.
- 2026-07-29: The focused gate exposed an undesirable deployable-to-deployable
  dependency from `hl-capture` to `hl-analytics`. Archive mechanics were moved
  without behavior changes into `crates/canonical-archive`; analytics re-exports
  that foundation and capture/inspection consume it directly. Strict Clippy,
  archive/analytics/capture/inspector tests, architecture checks, dependency
  exceptions, cargo-deny, workspace layout, and post-refactor E2E report
  `20260729T123149Z-26365` are green.
- 2026-07-29: Post-refactor soak report `20260729T123326Z-28210` is green:
  ten blocks, twenty acknowledgements, one restart, 16 seconds elapsed,
  24.9 MiB peak RSS, zero service output, verified archive, and clean shutdown.
  M5/M6 is ready for its local milestone commit.

## Rollback / Recovery

- If this fails: stop before advancing any real source cursor; preserve spool,
  archive, PostgreSQL rows, JetStream state, exact logs, and soak evidence for
  diagnosis.
- Safe fallback: revert only the uncommitted runtime slice or its isolated
  milestone commit. The verified archive milestone remains at `9532290`; the
  original user checkout remains untouched.
- Integration state uses a uniquely named local Compose project and explicit
  test-owned paths. Teardown does not delete volumes or archive/spool evidence
  unless the user explicitly authorizes that destructive cleanup.
- No external deployment, publication, credential issuance, public endpoint,
  private-key handling, trade, or production cursor advancement is authorized
  by this plan.
