# Immutable Parquet Archive and Manifest Chain

## Goal

- User-visible outcome: Alpha Desk can durably archive canonical blocks and
  byte-preserving source observations as immutable Parquet objects, verify the
  complete manifest chain before replay, and inspect or query a local archive
  without relying on ClickHouse, NATS, or a cloud service.
- How to see it working:
  - focused tests prove that orphan objects are invisible, corrupt objects stop
    reads before any row is returned, duplicate appends are idempotent, and a
    verified range reconstructs the original canonical blocks exactly;
  - `archive-inspect verify <archive-root>` prints the verified manifest,
    object, row, and block-range summary;
  - `archive-inspect count <archive-root>` reads committed Parquet data and
    reports the exact canonical-event count;
  - a committed clean-tree generated gate and the full local repository gate
    pass.

## Current State

- Relevant paths:
  - `crates/canonical-events/` owns validated canonical event and block
    envelopes, stable IDs, content hashes, and preserved Protobuf event bytes.
  - `crates/hl-protocol/` owns byte-preserving `SourceObservation`.
  - `crates/storage-ports/` is currently a dependency-free scaffold.
  - `services/hl-analytics/` is currently an immediately exiting binary with no
    library or archive implementation.
  - `services/hl-capture/` now owns the pure sequencer and emits logical
    `Commit` decisions, but does not persist a durable cursor.
  - `schemas/postgres/0001_capture_incidents.sql` already requires an archive
    manifest hash and receipt before cursor advancement.
- Existing behavior:
  - canonical events can round-trip through their exact V1 Protobuf envelope;
  - canonical blocks bind complete routing-critical event content but exclude
    source-only evidence from the canonical hash;
  - the spool already uses immutable close manifests and crash-safe publication
    patterns, but it is raw capture evidence rather than the replay archive;
  - there are no Arrow, Parquet, DataFusion, archive-manifest, archive-reader,
    compaction, or archive-inspection implementations.
- Constraints:
  - follow approved Truth Layer Task 7 and design section 14.1;
  - archive durability precedes sequencer cursor persistence and operational
    publication;
  - no S3/MinIO types leak into the storage port;
  - the first implementation is an operator-controlled local filesystem
    backend with a replaceable replication boundary;
  - canonical and raw bytes remain recoverable; indexed columns are not a
    substitute for the authoritative encoded envelope/payload;
  - all paths, manifests, sizes, counts, hashes, ranges, and schemas are
    validated fail closed;
  - no trading, credentials, public publication, or production mutation is in
    scope.

## Target State

- Desired behavior:
  - `storage-ports` defines validated archive receipts, verified manifests,
    raw-observation records, range iterators, and a synchronous
    `CanonicalArchive` boundary with no backend-specific types.
  - The local backend writes data to a temporary same-filesystem file, syncs
    it, validates the complete Parquet object, atomically publishes the object,
    atomically publishes an immutable canonical manifest, then atomically
    advances a small validated `CURRENT` pointer and syncs the parent
    directory.
  - Readers snapshot and verify the full manifest chain, schemas, object paths,
    file sizes, SHA-256 hashes, row counts, block ranges, block hashes, and
    source watermarks before yielding any canonical block.
  - Canonical-event Parquet rows retain the exact encoded canonical envelope
    plus query columns. Empty canonical blocks remain reconstructable from
    manifest block descriptors.
  - Raw-observation Parquet rows retain the exact source payload plus source,
    cursor, receive-time, parser, warning, and content-hash metadata.
  - Re-appending an already archived matching block returns the existing
    receipt; the same height with different canonical content fails as
    corruption/divergence.
  - Compaction is a verified immutable generation transition. It preserves
    stable `(block_height, transaction_index, canonical_event_index)` order,
    row count, rolling content hash, and old manifest reachability until a
    separate retention policy authorizes deletion.
  - Committed JSON schema descriptions freeze the Arrow/Parquet field names,
    types, nullability, metadata, and semantic version. Their deterministic
    fingerprints are embedded in manifests and Parquet metadata.
- Non-goals:
  - no MinIO/S3 replication, remote credentials, retention deletion, or
    disaster-recovery drill in this milestone;
  - no JetStream publication, sequencer cursor coordinator, or long-running
    service wiring until archive receipts are proven;
  - no ClickHouse loading or derived analytical tables;
  - no arbitrary SQL surface in the initial inspection CLI; the required
    reviewed count/readability smoke is sufficient and avoids an unsafe
    free-form local query contract.

## Risks and Failure Modes

- A manifest can become visible before every referenced object is durable,
  allowing a cursor to advance past data that cannot be replayed.
- A reader can stream early rows and discover corruption later, creating
  partial state mutation before verification fails.
- A one-row-per-event representation can silently lose empty blocks or
  source-block evidence needed to reconstruct a `BlockEnvelope`.
- JSON serialization, map iteration, timestamps, temporary names, or Parquet
  writer metadata can make manifest IDs and compaction output nondeterministic.
- A path in a manifest can traverse, escape the archive root, follow a symlink,
  or alias another object.
- Concurrent writers can fork the manifest chain or replace one another's
  `CURRENT` pointer.
- Exact fixed-point and unsigned values can be narrowed or converted through
  floating point.
- Compaction can preserve row count while reordering, duplicating, or changing
  authoritative envelope bytes.
- Arrow/DataFusion dependency selection can exceed the pinned Rust toolchain or
  violate the supply-chain policy.

## Milestones

### M1. Freeze archive contracts and red tests

- Goal: make durability, identity, path, schema, and replay semantics explicit
  before implementing I/O.
- Files / systems:
  - `crates/storage-ports/src/archive.rs`
  - `crates/storage-ports/src/lib.rs`
  - `crates/storage-ports/Cargo.toml`
  - `services/hl-analytics/src/lib.rs`
  - `services/hl-analytics/tests/archive.rs`
  - `schemas/parquet/canonical-events-v1.json`
  - `schemas/parquet/raw-observations-v1.json`
- Changes:
  - define validated receipt/manifest/range-reader contracts and stable archive
    error reason codes;
  - commit exact schema documents;
  - add tests for invisible orphan objects, corruption-before-yield,
    idempotency, conflicting duplicate height, empty blocks, ordered range
    replay, unsafe paths/symlinks, and manifest-chain breaks;
  - initially reference the absent local backend so the test target fails for
    the intended reason.
- Verification:
  - `cargo +1.97.1 test -p hl-analytics --test archive --locked --offline`
- Expected result: first run fails because the archive implementation is
  absent; after the contract slice compiles, behavior tests remain red until
  their owning vertical milestone is implemented.

### M2. Implement canonical Parquet object and atomic manifest publication

- Goal: archive a canonical block only after its exact Parquet object and
  immutable manifest are durable and verified.
- Files / systems:
  - `services/hl-analytics/src/archive/mod.rs`
  - `services/hl-analytics/src/archive/schema.rs`
  - `services/hl-analytics/src/archive/writer.rs`
  - `services/hl-analytics/src/archive/manifest.rs`
  - `services/hl-analytics/tests/archive.rs`
  - workspace and package dependency manifests
- Changes:
  - pin one compatible Arrow/Parquet family;
  - encode exact event-envelope bytes with query columns and block descriptors;
  - implement same-directory temporary publication, file and directory sync,
    immutable content-addressed manifests, and a compare-under-lock `CURRENT`
    transition;
  - validate the written object before returning `ArchiveReceipt`;
  - preserve orphan objects on injected pre-manifest failure so readers prove
    they ignore them.
- Verification:
  - focused atomicity/idempotency/empty-block tests;
  - strict package Clippy.
- Expected result: only fully manifested blocks are visible and every returned
  receipt binds block height, canonical block hash, object hash, manifest hash,
  schema fingerprint, and durable timestamp.

### M3. Verify complete chains and reconstruct ranges before yielding

- Goal: make replay fail closed before consumers can mutate state.
- Files / systems:
  - `services/hl-analytics/src/archive/reader.rs`
  - `services/hl-analytics/src/archive/manifest.rs`
  - `services/hl-analytics/tests/archive.rs`
- Changes:
  - snapshot `CURRENT`, load and canonicalize the reachable manifest chain, and
    validate every referenced object before creating the public iterator;
  - reject missing/corrupt/aliased/symlinked objects, schema drift, count/range
    disagreement, block-hash disagreement, chain forks, and unavailable ranges;
  - reconstruct exact `BlockEnvelope` values including empty blocks and sorted
    source-block evidence.
- Verification:
  - corruption-before-yield, unsafe-path, chain-break, and exact range
    round-trip tests.
- Expected result: any archive defect returns a typed error before the first
  block is yielded; a valid range equals the original validated block sequence.

### M4. Archive raw source observations without losing bytes

- Goal: retain the raw-first evidence needed to rebuild canonical mappings.
- Files / systems:
  - `services/hl-analytics/src/archive/raw_writer.rs`
  - `services/hl-analytics/src/archive/schema.rs`
  - `services/hl-analytics/tests/archive.rs`
  - `schemas/parquet/raw-observations-v1.json`
- Changes:
  - write source identity/version/class, cursor epoch/offset, receive times,
    parser version, exact payload bytes, BLAKE3 content hash, and deterministic
    warning encoding;
  - partition by source/date/hour without accepting caller-provided paths;
  - verify a byte-exact raw observation round trip and content hash.
- Verification:
  - focused raw round-trip and schema-drift tests.
- Expected result: no accepted raw observation loses or normalizes source
  bytes, and manifests bind exact source watermarks.

### M5. Add verified idempotent compaction generations

- Goal: reach production-sized Parquet files without weakening immutability or
  replay identity.
- Files / systems:
  - `services/hl-analytics/src/archive/compactor.rs`
  - `services/hl-analytics/tests/archive.rs`
- Changes:
  - select only contiguous verified input objects in one compatible partition;
  - write a new immutable generation in stable canonical order;
  - compare row count, block descriptors, and rolling authoritative-envelope
    hash before switching `CURRENT`;
  - return the existing generation when the same compaction request is
    repeated;
  - keep prior manifests and objects reachable; implement no deletion.
- Verification:
  - idempotency, order, injected-crash, and mismatched-input tests.
- Expected result: verified compacted and uncompacted range reads are byte- and
  block-hash equivalent.

### M6. Add an inspection tool and independent Parquet readability smoke

- Goal: let an operator verify and inspect an archive without starting a
  service or trusting manifest claims alone.
- Files / systems:
  - `tools/archive-inspect/Cargo.toml`
  - `tools/archive-inspect/src/lib.rs`
  - `tools/archive-inspect/src/main.rs`
  - `tools/archive-inspect/tests/cli.rs`
  - workspace manifest
  - `docs/formats/archive-manifest-v1.md`
  - `docs/DEVELOPMENT.md`
  - `README.md`
- Changes:
  - add strict `verify` and `count` commands with bounded, single-line summary
    output and non-zero typed failures;
  - independently open committed Parquet objects through the pinned query
    engine/read path and compare its row count with the verified manifest;
  - document format, atomicity, chain verification, recovery, and limitations.
- Verification:
  - CLI success/failure tests;
  - manual fixture archive verification and count.
- Expected result: the tool reports exact chain/object/row/range hashes and
  detects one-byte object or manifest corruption.

### M7. Close the archive milestone

- Goal: produce one reviewable commit with fresh source, format, runtime, and
  OSS evidence.
- Files / systems:
  - archive implementation, schemas, tool, docs, changelog, status, generated
    checks, and active plans.
- Changes:
  - add deterministic fixture regeneration to `just generated`;
  - update status without claiming the long-running product or Stage 1 gate;
  - review every new dependency and intentional file;
  - commit only this milestone.
- Verification:
  - `cargo +1.97.1 fmt --all -- --check`
  - `cargo +1.97.1 clippy -p storage-ports -p hl-analytics -p archive-inspect --all-targets --all-features --locked --offline -- -D warnings`
  - `cargo +1.97.1 test -p hl-analytics --test archive --locked --offline`
  - `cargo +1.97.1 test -p archive-inspect --locked --offline`
  - `cargo +1.97.1 deny --locked --offline check`
  - `just verify`
  - `just oss-audit`
  - `git diff --check`
  - post-commit `just generated`
- Expected result: the clean worktree contains one intentional archive commit
  and detached regeneration reproduces its schemas, fixture archive, manifest
  hashes, reads, and operator summaries.

## Verification

- `cargo +1.97.1 test -p hl-analytics --test archive --locked --offline`
- `cargo +1.97.1 test -p archive-inspect --locked --offline`
- `cargo +1.97.1 clippy -p storage-ports -p hl-analytics -p archive-inspect --all-targets --all-features --locked --offline -- -D warnings`
- `cargo +1.97.1 deny --locked --offline check`
- `just verify`
- `just oss-audit`
- `git diff --check`
- Manual smoke:
  1. create a fresh temporary archive from the committed canonical fixture;
  2. verify it and count events with `archive-inspect`;
  3. copy it, corrupt one byte in a referenced object, and confirm both verify
     and count fail before returning data;
  4. confirm the original archive still verifies and the repository worktree
     remains clean after the fixture command.

## Decision Log

- 2026-07-29: Implement the approved local-filesystem backend first. The port
  contains no S3 types, so later operator-controlled replication does not
  change canonical archive semantics.
- 2026-07-29: Store exact canonical Protobuf envelope bytes and raw source
  payload bytes as authoritative columns. Derived query columns are validated
  indexes and cannot replace the encoded records.
- 2026-07-29: Describe empty blocks in manifests because a one-row-per-event
  Parquet object otherwise cannot reconstruct them.
- 2026-07-29: Verify the full requested archive snapshot before exposing an
  iterator. Streaming validation after yielding would permit partial reducer
  mutation.
- 2026-07-29: Keep prior generations and implement no retention deletion in
  this milestone. Deletion requires the later replication and recovery policy.
- 2026-07-29: Start with strict `verify` and `count` commands. A free-form SQL
  CLI is not needed to prove Parquet readability and would create a broader
  input/resource-control surface.
- 2026-07-29: Treat an empty directory as an operator-verification failure.
  A structural zero-count success could be mistaken for evidence that capture
  data was actually archived.
- 2026-07-29: Raw objects are strictly hour-bounded. Each raw receive-hour has
  an immutable append-only partition chain, and the source catalog duplicates
  the ordered batch references so verification can reject either a missing
  partition generation or catalog/partition disagreement.
- 2026-07-29: Pin DataFusion `54.1.0` with Arrow/Parquet `58.4.0`. Keep its
  exact `paste`, `tiny-keccak`, `zlib-rs`, `foldhash`, and transition duplicate
  exceptions narrow, documented, and inverse-path checked.

## Progress Log

- 2026-07-29: Sequencer milestone `a56352f` passed full local verification,
  exact PostgreSQL migration smoke, OSS audit, and detached clean-tree
  regeneration.
- 2026-07-29: Reviewed approved Truth Layer Task 7, design section 14.1,
  current canonical/source contracts, storage-port scaffold, and analytics
  scaffold.
- 2026-07-29: M1–M5 implemented: backend-neutral archive ports, frozen schemas,
  atomic content-addressed canonical/raw publication, complete catalog and
  partition chain verification, corruption-before-yield range replay,
  exact empty-block/raw-byte recovery, conflicting-range rejection, bounded
  reads, and idempotent retained-generation canonical compaction.
- 2026-07-29: M6 implemented: `archive-inspect verify` performs reachable
  manifest/object verification, `count` independently reads canonical Parquet
  through DataFusion, empty roots fail closed, and focused CLI/integration
  tests pass.
- 2026-07-29: M7 implementation complete pending the full local gates and
  milestone commit. The synthetic fixture deterministically regenerates a
  three-block compacted canonical archive plus a three-observation raw
  partition; `just generated` compares all bytes and both operator summaries.
- 2026-07-29: One of two full `just verify` runs exposed a low-frequency
  pre-existing stage-gate test failure: its final mutation test produced a
  normal report rather than the expected repository-phase lifecycle report.
  The exact test then passed in isolation and the complete 33-test target
  passed three consecutive 16-thread runs. The safety assertion remains
  strict; its diagnostic now includes the full unexpected report. This is
  tracked as nondeterministic local test risk, not waived CI.
- 2026-07-29: Current local closeout evidence is green: strict whole-workspace
  Clippy, architecture and unsafe checks, dependency policy, one complete
  `just verify` pass, a later fresh `just test` pass after the diagnostic
  change, Swift tests, `just oss-audit` over 386 files, deterministic fixture
  byte comparison, and manual one-byte corruption rejection by both operator
  commands. The optimized operator binary verifies/counts the fixture; its
  unstripped macOS arm64 footprint is 79 MiB. Debug linking emits an Apple
  compact-unwind size warning, while the optimized build does not.

## Rollback / Recovery

- If this fails: stop before connecting archive receipts to the sequencer
  cursor; preserve failing archive fixtures and exact object/manifest bytes for
  diagnosis.
- Safe fallback: revert only the uncommitted archive slice or its isolated
  milestone commit. The preceding clean sequencer commit is `a56352f`; raw spool
  evidence and the original checkout remain untouched.
- Temporary test archives are created under test-owned temporary directories.
  Tests and tools never recursively delete a caller-provided archive root.
- No manifest/object retention deletion, external write, credential use, or
  production cursor advancement is authorized in this plan.
