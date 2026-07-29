# Contiguous Canonical Sequencer Foundation

## Goal

- User-visible outcome: Alpha Desk can deterministically accept qualified
  committed and provisional canonical blocks, stop on gaps or divergence, and
  explain the exact recovery or quarantine action required.
- How to see it working: focused state-machine tests reproduce the approved
  primary-gap, independent-source recovery, duplicate, divergence, provisional,
  capacity, and retained-history cases with stable decisions and reason codes.

## Current State

- Relevant paths:
  - `crates/canonical-events/` owns validated event and block envelopes.
  - `crates/hl-protocol/` owns fail-closed source trust and publication lanes.
  - `services/hl-capture/` owns source adapters and durable local spooling.
  - `crates/canonical-ledger/` remains a bootstrap for later account-state
    reduction and is not the capture sequencer.
- Existing behavior: capture preserves and classifies source evidence, and the
  canonical mapper produces stable block/event hashes, but no component yet
  enforces cross-source height continuity.
- Constraints:
  - Only a validated committed-candidate admission may advance committed state.
  - Provisional blocks never mutate committed state.
  - A stable height or event identity with different content is divergence.
  - Pending and retained histories must be bounded.
  - PostgreSQL/archive/NATS I/O is not invented in the pure state machine.

## Target State

- Desired behavior:
  - A synchronous deterministic sequencer owns committed/provisional
    watermarks, bounded pending candidates, bounded recent committed hashes,
    outstanding gap ranges, and a fail-closed divergence latch.
  - Matching evidence deduplicates; conflicting evidence produces a
    reproducible quarantine incident and stops committed advancement.
  - Old observations beyond retained in-memory history request durable archive
    verification instead of being accepted or rejected speculatively.
  - SQL and runbooks freeze incident/cursor persistence and operator recovery
    contracts ahead of the archive implementation.
- Non-goals:
  - No PostgreSQL client, Parquet writer, historical network adapter, or NATS
    publisher is added in this slice.
  - A logical `Commit` decision is not an archive durability receipt; the
    durable cursor integration remains part of the archive/runtime milestone.

## Risks and Failure Modes

- A source can be accidentally promoted despite an incompatible admission or
  confirmation class.
- Committing future heights while a gap is open can silently make replay
  incomplete.
- Unbounded pending/history state can turn source disorder into memory
  exhaustion.
- A late conflict at an already committed height can be mislabeled as a normal
  duplicate.
- Incident identifiers can become nondeterministic or leak source payloads.

## Milestones

### M1. Freeze state-machine behavior with failing tests

- Goal: encode the approved continuity and divergence examples plus boundary
  failures before implementation.
- Files / systems: `services/hl-capture/tests/sequencer.rs`.
- Changes: add deterministic block fixtures and assertions for gap recovery,
  duplicate suppression, provisional isolation, divergence latching, admission
  rejection, bounded capacity, and archive verification of evicted history.
- Verification: `cargo +1.97.1 test -p hl-capture --test sequencer --locked --offline`.
- Expected result: compilation/test failure because the sequencer API does not
  exist.

### M2. Implement the bounded pure sequencer

- Goal: make every accepted candidate produce explicit, deterministic
  decisions without I/O or ambient time.
- Files / systems:
  - `services/hl-capture/src/sequencer/mod.rs`
  - `services/hl-capture/src/sequencer/watermark.rs`
  - `services/hl-capture/src/sequencer/gap.rs`
  - `services/hl-capture/src/sequencer/divergence.rs`
  - `services/hl-capture/src/quarantine.rs`
  - `services/hl-capture/src/lib.rs`
  - `services/hl-capture/Cargo.toml`
- Changes: add validated candidates, bounded state/configuration, gap
  coalescing, source-evidence merging, retained-hash verification decisions,
  deterministic incident IDs, reason codes, and a permanent red latch after
  divergence.
- Verification: focused test plus package Clippy with warnings denied.
- Expected result: all focused tests pass and no asynchronous or storage API
  enters the pure state machine.

### M3. Freeze persistence and recovery contracts

- Goal: define how later orchestration records critical incidents and advances
  durable cursors only after archive durability.
- Files / systems:
  - `schemas/postgres/0001_capture_incidents.sql`
  - `docs/runbooks/committed-gap.md`
  - `docs/runbooks/source-divergence.md`
  - repository status/development documentation.
- Changes: add append-only evidence metadata, explicit resolution fields,
  archive-bound cursor constraints, recovery ordering, suppression rules, and
  reproducible inspection commands.
- Verification: formatting, Markdown/SQL review, OSS audit.
- Expected result: the persistence boundary is documented without claiming a
  database adapter exists.

### M4. Repository verification and commit

- Goal: retain a clean reviewable milestone.
- Files / systems: all intentional files above plus generated/workspace checks.
- Changes: update progress/evidence, inspect the complete diff, and commit only
  the sequencer slice.
- Verification:
  - `just verify`
  - `just generated`
  - `just oss-audit`
  - `git diff --check`
- Expected result: clean branch with detached-tree determinism and the next
  archive milestone unblocked.

## Verification

- `cargo +1.97.1 test -p hl-capture --test sequencer --locked --offline`
- `cargo +1.97.1 clippy -p hl-capture --all-targets --all-features --locked --offline -- -D warnings`
- `just verify`
- `just generated`
- `just oss-audit`
- Manual smoke: feed heights 100, 101, 103, then matching independent 102;
  confirm decisions request only 102 and later commit 102 and 103 in order.

## Decision Log

- 2026-07-29: Keep the pure sequencer in `hl-capture`; `canonical-ledger`
  remains the deterministic account/state reducer described by the approved
  repository architecture.
- 2026-07-29: Bound both future candidates and recent committed hashes.
  Candidates older than retained memory require durable archive verification.
- 2026-07-29: Treat a logical commit decision and durable cursor advancement as
  separate contracts. Archive-before-cursor ordering will be implemented with
  the immutable archive rather than simulated here.

## Progress Log

- 2026-07-29: Plan created after canonical upcast/inspection milestone
  `b4e490c` passed full and detached-tree gates.
- 2026-07-29: Captured the expected RED compile boundary, then implemented the
  bounded pure sequencer. Thirteen focused scenarios now pass, including
  full-buffer gap recovery, same-source raw-hash conflict, provisional
  isolation during a committed red latch, evicted-history verification, and
  the terminal `u64::MAX` height.
- 2026-07-29: Added PostgreSQL incident/cursor schema contracts and committed
  gap/source-divergence runbooks with explicit current implementation limits.
- 2026-07-29: Sequencer review found that the prior block projection did not
  bind market/account routing metadata and buffered matching sources did not
  merge event-level evidence. Added failing regressions, extended the V1
  canonical content projection, and implemented deterministic event-evidence
  reconciliation with same-locator conflict rejection.
- 2026-07-29: Full `just verify` passed, including all-features workspace
  Clippy, architecture/unsafe checks, dependency policy, all Rust tests and
  doc-tests, and Swift package tests. The exact pinned PostgreSQL migration
  smoke passed and the OSS audit reported `PASS files=337`.
- 2026-07-29: Next: review, commit, and reproduce the milestone from a detached
  clean tree.

## Rollback / Recovery

- If this fails: preserve the failing sequence and keep the previous clean
  canonical milestone at `b4e490c`.
- Safe fallback: remove only the new sequencer slice from this isolated branch;
  do not change the original dirty checkout or weaken source-admission rules.
