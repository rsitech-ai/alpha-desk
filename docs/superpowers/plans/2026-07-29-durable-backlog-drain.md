# Durable Backlog Drain Implementation Plan

**Goal:** Keep primary-source acquisition durable and bounded while canonical
processing, PostgreSQL, or NATS is unavailable, then resume canonical
processing deterministically from the verified source spool.

**Authority boundary:** This remains a read-only capture and interpretation
system. It does not add trading, signing, private-key access, remote
publication, or live-source qualification.

**Spec contract:** The source spool is the first and only source
acknowledgement boundary. A downstream outage makes canonical state stale and
health degraded/non-ready, but it must not stop capture while the configured disk reserve
can be preserved. Recovery reads exact source bytes from the spool, never from
an invented substitute. The affected canonical watermark remains contiguous;
later observations may accumulate durably without being applied out of order.

## Progress

- [x] Stream spool verification, replay, and raw archival with bounded record
  and byte allocation.
- [x] Split local acquisition from canonical drain; accepted observations are
  fsynced and acknowledged without a PostgreSQL, JetStream, or coordinator
  dependency.
- [x] Drain verified cursor ranges from durable PostgreSQL progress and advance
  only after committed or identical-duplicate outcomes.
- [x] Recreate bounded PostgreSQL and JetStream sessions after storage or
  transport failure, including operation timeouts and cancellation-aware
  retry.
- [x] Emit non-ready yellow status with stable reasons while downstream is
  unavailable, retaining the last verified durable cursor when possible.
- [x] Prove real disposable NATS and PostgreSQL pause/recovery with exact
  five-record spool/raw/block/publication parity and
  `live_source_qualified=false`.
- [ ] Add explicit backlog size/oldest-cursor and percentage disk-health
  metrics.
- [ ] Replace fixed retry delay with bounded exponential backoff plus jitter
  that remains deterministic under tests.
- [ ] Add the SIGKILL boundary matrix, multi-hour soak, host-restart proof, and
  production TLS/identity/replicated JetStream qualification.

## Architecture

The runtime is split into two independently owned loops:

1. **Acquisition loop:** opens the source adapter and `SourceSpool`, checks disk
   capacity, fsyncs each committed observation, acknowledges only its spool
   durability receipt, rotates and archives verified raw segments, and keeps
   running without PostgreSQL or NATS.
2. **Drain loop:** connects to canonical dependencies, obtains the next
   expected canonical height, streams verified spool records from that point,
   maps and sequences them, and performs archive/journal/publication/cursor
   coordination. Transient downstream failure closes the current dependency
   session, marks health degraded, backs off within bounded policy, and retries
   from durable progress.

The spool manifest cursor ranges are the durable backlog index. No second
unverified queue or mutable “last processed” sidecar is authoritative. An
in-process bounded notification may reduce latency, but losing a notification
cannot lose data because the drain loop rescans from durable progress.

## Slice 1: Bounded spool reads

- Add an iterator-style spool reader that allocates at most one bounded record
  body at a time and reports the next byte offset.
- Distinguish strict closed-segment EOF from a concurrently written incomplete
  active tail without treating a complete active prefix as corruption.
- Refactor spool inspection, manifest verification, source replay, and raw
  archival away from `read_all`.
- Bound raw Parquet batches by both record count and uncompressed payload bytes,
  while retaining hour partitions, contiguous cursors, and spool hash binding.
- Cap segment targets at the design's 512 MiB upper recommendation instead of
  accepting an unqualified 4 GiB segment; retain smaller development segments
  and benchmark 128–512 MiB before production selection.

## Slice 2: Local-only acquisition

- Extract the adapter/spool/raw-archive path into an owned task that has no
  PostgreSQL, JetStream, or canonical-coordinator dependency.
- Start the adapter from the last verified durable spool cursor.
- Continue accepting and fsyncing observations when the drain loop is absent or
  retrying, subject only to source integrity, local archive integrity, and disk
  reserve.
- Keep raw archival idempotent on rotation and graceful shutdown.
- Surface backlog size, oldest pending cursor, disk capacity, and acquisition
  health without leaking paths or credentials.

## Slice 3: Reconnecting deterministic drain

- Build dependency sessions through a bounded reconnect supervisor rather than
  requiring PostgreSQL and NATS before source acquisition starts.
- Locate the first relevant spool segment from manifest cursor ranges and
  stream forward from the durable canonical height.
- Treat downstream transport/storage failures as retryable session failures;
  treat mapping, divergence, corruption, and invariant failures as latched
  fail-closed data incidents.
- Use bounded exponential backoff with cancellation-aware waits and explicit
  health transitions.
- Remove or put `queue_capacity` on a real bounded notification/batch boundary;
  no accepted configuration field may remain operationally inert.

## Slice 4: Fault evidence

- Add deterministic tests proving a stopped downstream cannot prevent spool
  growth or source acknowledgements.
- Add restart tests proving duplicate drain is idempotent and cursor-bound.
- Add self-contained E2E cases for NATS outage/recovery and PostgreSQL
  outage/recovery, with exact spool/raw/canonical/publication parity.
- Add SIGKILL failpoints at spool append, raw close/archive, canonical archive,
  journal, publish, acknowledgement, and cursor advancement.
- Retain machine-readable evidence with `live_source_qualified=false`.

## Completion checks

- Peak memory remains bounded by configuration rather than retained segment
  size.
- Acquisition continues during a bounded downstream outage until disk policy
  stops it.
- Canonical progress remains contiguous and catches up exactly once after
  dependency recovery.
- Cancellation owns and joins acquisition, drain, dependency drivers, and
  status tasks.
- Strict Rustfmt, Clippy, full workspace tests, source E2E, outage E2E, soak,
  archive/spool verification, OSS audit, and secret scan pass.
