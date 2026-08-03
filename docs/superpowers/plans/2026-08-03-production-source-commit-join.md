# Production Source Qualification and Committed Trade Join

> **Execution rule:** Use `superpowers:subagent-driven-development` or
> `superpowers:executing-plans`. Each implementation milestone uses
> red-green-refactor, one writer, an independent requirements review, an
> independent quality review, and a scoped local commit. Do not promote a
> synthetic fixture, a caller-provided version string, or an auxiliary event by
> changing its confirmation enum.

## Goal

- User-visible outcome: `hl-capture run` durably captures a qualified
  Hyperliquid node's committed `replica_cmds` blocks and block-batched trade
  output, constructs committed trade events only after an exact source/commit
  join, survives crashes and restarts without duplicate effects, and retains a
  machine-verifiable evidence report.
- How to see it working: an operator starts the read-only capture service with
  one qualified node instance, observes both raw streams advance, stops or
  crashes the service at any tested persistence boundary, restarts it, and
  receives the same canonical block/event/state hashes as an uninterrupted
  run. Missing, ambiguous, divergent, unqualified, or schema-drifted evidence
  turns health RED and prevents canonical cursor advancement.
- Completion boundary: synthetic and generated-corpus checks establish
  `repo-ready`; only a retained, byte-exact, same-build operator recording plus
  restart/replay evidence can establish `runtime-proven` for this source path.
  Independent-source reconciliation, Stage 1, Stage 2, deployment, and public
  release remain separate gates.
- Authority boundary: local code, tests, documentation, fixtures that are
  legally redistributable, and local evidence may be changed. Do not install
  or operate a live node, publish recordings, push, open a PR, deploy, trade,
  or handle private keys without separate authority.

## Current State

- Relevant paths:
  - `services/hl-capture/src/source_runtime.rs`
  - `services/hl-capture/src/committed_pipeline.rs`
  - `services/hl-capture/src/adapters/node_files.rs`
  - `services/hl-capture/src/adapters/node_stream.rs`
  - `services/hl-capture/src/spool/`
  - `crates/hl-protocol/src/node/v1.rs`
  - `crates/hl-protocol/src/trust.rs`
  - `crates/canonical-events/src/node_mapping.rs`
  - `crates/canonical-events/src/block.rs`
  - `crates/canonical-archive/src/raw.rs`
  - `crates/storage-ports/src/archive.rs`
  - `services/hl-core/src/lib.rs`
  - `services/hl-core/src/main.rs`
  - `fixtures/source/node-v1/`
  - `tools/ci/capture-e2e.sh`
- Existing behavior:
  - `NodeBlockDirectorySource` captures committed transaction blocks, fsyncs
    them to a source spool before acknowledgement, and drains them into the
    committed pipeline.
  - The committed mapper accepts only blocks with no action bundles. It emits
    an empty committed block with one committed source hash and rejects every
    action-bearing block.
  - `NodeLineFileSource` correctly parses complete lines, preserves bytes,
    handles partial writes and rotation, and has explicit durable
    acknowledgement, but `hl-capture run` never instantiates it.
  - The auxiliary trade mapper preserves participant anchors and source
    evidence, but assigns trade-only indices and always emits
    `ProvisionalSource`. Its only non-test caller is `canonical-inspect`.
  - `BlockCandidate::try_new` is intentionally single-source and cannot carry
    a committed-block-plus-trade proof.
  - `SpoolBacklog`, `RawObservationBatch`, and raw archive range verification
    assume `offset + 1`; node-line cursors are byte offsets and cannot reuse
    that continuity contract.
  - The coordinator already orders canonical archive, durable publication
    plan, publish/acknowledge, and cursor advance. A join must finish before
    coordinator entry because a late trade cannot amend an archived block.
  - `hl-core::apply_block_durably` provides an atomic state-apply primitive,
    but the service executable does not consume committed blocks.
  - Existing source fixtures are explicitly
    `normalized-official-documentation-examples` with
    `production_recording = false`. Existing capture/replay evidence is
    synthetic and correctly reports live/deployed qualification false.
- Constraints:
  - V1 is read-only. No signer, private key, order placement, custody, or
    execution route.
  - Preserve the trust matrix: auxiliary node output remains reconciliation
    evidence until an exact committed join constructs a new committed event.
  - The full committed `actions-and-responses` match projection and the
    exact zero-trade completion behavior must be proven by a retained recording
    from an exact node build. Do not infer undocumented fields.
  - The same physical node instance may supply both halves of the primary join;
    that does not count as independent reconciliation.

## Target State

- Desired behavior:
  - A digest-pinned `NodeSourceQualificationV1` binds chain ID, stable node
    instance ID, distinct committed/trade source IDs, node repository commit,
    binary SHA-256 and signature evidence, exact runtime arguments, required
    output flags, parser/mapper/catalog versions and hashes, qualification
    manifest digest, time-normalization rule, and block-completion rule.
  - Unsupported or caller-invented profiles fail startup with a stable
    `source_join.unqualified_source_profile` reason.
  - Both observations are retained in independent hash-chained spools and raw
    archives before either source is acknowledged.
  - A durable pending-join index uses
    `(chain_id, node_instance_id, block_height)` and preserves each source's
    native cursor separately from the local monotonic spool sequence.
  - A versioned committed parser produces an ordered transaction manifest and
    ordered full match commitments from the retained committed response. The
    auxiliary trade hash is accepted as transaction identity only when the
    qualified source contract and real corpus prove that relation.
  - `CommittedTradeJoinV1` requires exact height, normalized block time,
    transaction identity/order, complete match count/order, market,
    participants, order IDs, optional IDs, price, quantity, and start-position
    equality. Timestamp proximity, hash membership alone, and an ordered
    subsequence are insufficient.
  - A successful join constructs a new committed block atomically. Events use
    committed execution order, fixed evidence order (committed first,
    auxiliary second), and both raw hashes. Lifecycle times are replay-stable:
    `observed_at` is the persisted auxiliary observation receive wall time,
    `ingested_at` is the maximum persisted receive wall time of the two source
    observations, and `canonicalized_at` equals `ingested_at`. Actual processing
    and durability times belong to operational receipts/telemetry, not the
    canonical event bytes.
  - A create-once `JoinDecisionV1` written before canonical archive binds the
    profile and recording-manifest digests, both source/cursor/content hashes,
    ordered transaction/match-manifest digest, event IDs, payload hashes,
    canonical block hash, counts, and disposition.
  - A separate create-once `JoinedBlockFinalizationV1` written only after the
    canonical archive append binds the join-decision digest to the archive's
    canonical block hash, object SHA-256, manifest SHA-256, and schema
    fingerprint. If the archive exists but finalization does not, restart must
    verify an idempotent archive result and complete finalization before
    publication.
  - Exact replay is idempotent. Conflicting source duplicates, multiple
    candidates, mismatches, missing source-native completion, schema drift, or
    join-decision/finalization divergence quarantine the entire block and hold
    the canonical cursor.
  - `hl-core` consumes committed blocks, restores the canonical reducer,
    applies each block durably, verifies the receipt, and acknowledges only
    after the state commit.
  - Evidence tooling compares uninterrupted and restarted runs and reports
    truth-qualified booleans without caller overrides.
- Stable error precedence:
  1. Invalid or unqualified configuration.
  2. Raw parse or schema failure.
  3. Committed cursor, header, or continuity failure.
  4. Conflicting logical-source duplicate.
  5. Missing or ambiguous join candidate.
  6. Chain, height, or block-time mismatch.
  7. Transaction identity or order mismatch.
  8. Match count, order, or field mismatch.
  9. Catalog, fixed-point, or domain rejection.
  10. Canonical block contract rejection.
- Non-goals:
  - No timestamp-window join, post-publication amendment, or confirmation flip.
  - No source qualification from configuration strings, environment booleans,
    fixture filenames, or an unreviewed operator assertion.
  - No weakening of the existing single-source candidate constructor.
  - No redistribution of private/operator node data without an explicit
    redistribution classification.
  - No independent-source, trading, deployment, Stage 1, or Stage 2 promotion
    as part of this plan.

## Risks and Failure Modes

- The published node documentation does not freeze every
  `actions-and-responses` field needed for exact match projection. Implementing
  from a guessed or third-party shape would create false committed truth.
- The auxiliary stream can lag, rotate, restart, or leave a partial line while
  committed files advance. Wall-clock timeouts may make health RED but cannot
  change deterministic replay disposition.
- Byte-offset cursors are not contiguous integers. Reusing block-height range
  code could skip or duplicate data after rotation.
- A block may contain non-trade transactions between trade transactions and a
  transaction may contain multiple matches. Trade-only array order cannot
  assign canonical transaction or full event indices.
- Identical-looking legitimate matches must not be deduplicated. Match ordinal
  is part of committed identity.
- Accepting an empty committed block before an explicit empty trade batch or a
  qualified block-complete marker would make late evidence unrepairable.
- A crash after one source fsync, raw archive, join decision, canonical archive,
  finalization, publication, state apply, or cursor advancement can expose
  split-brain state unless every boundary is replayed and verified.
- A single physical node's two outputs are correlated evidence, not
  operationally independent evidence.
- A real recording may contain personal/operator-sensitive material or unclear
  redistribution rights. Keep it private by default and publish only derived,
  reviewed fixtures when permitted.

## Milestones

### M1. Freeze qualification and byte-recording envelopes

- Goal: represent source qualification and retained raw recording claims
  without freezing any unobserved committed-match or zero-trade semantics.
- Files / systems:
  - Create `crates/hl-protocol/src/node/qualification.rs`
  - Create focused tests under `crates/hl-protocol/tests/`
  - Create `docs/contracts/node-source-qualification-v1.md`
  - Create `docs/runbooks/operator-node-recording.md`
- Changes:
  - Define separate typed SHA-256 and BLAKE3 digests, bounded node-instance and
    artifact/version identities, a source group with distinct committed/trade
    source IDs, opaque native cursor evidence, immutable recording-file
    descriptors, and a canonical recording/qualification manifest.
  - Decode bounded bytes through private `deny_unknown_fields` wire structs,
    validate all invariants, re-encode byte-for-byte, and compute the manifest
    digest internally.
  - Treat the decoded manifest as an untrusted caller claim. Bind qualification
    to an internal digest-pinned registry entry and return only an opaque,
    private-field `QualifiedNodeSourceV1` token; the production registry remains
    empty until M4 passes.
  - Do not define transaction manifests, match commitments, source-native
    completion semantics, join proofs, or final receipts in this milestone.
  - Document exact private capture metadata, file hashing, redistribution
    classification, range closure, and corpus coverage requirements.
- Verification:
  - Focused RED tests first for self-declared profiles, unknown fields,
    noncanonical encodings, digest mismatch, wrong flags/build/catalog, distinct
    source identity, cursor evidence preservation, algorithm confusion, and
    bounded inputs. A module-private registry fixture proves the opaque-token
    success path without exposing a public registry constructor.
  - `cargo +1.97.1 test -p hl-protocol --locked --offline`
- Expected result: qualification/recording envelopes compile, serialize
  deterministically, reject caller-forged qualification, and qualify no
  production build. Existing trusted empty-block admission remains unchanged;
  this token is mandatory only for the joined-trade path introduced later.

### M2. Generalize raw retention without weakening native cursor semantics

- Goal: durably retain node-line observations whose native byte offsets are not
  `offset + 1`.
- Files / systems:
  - `services/hl-capture/src/spool/`
  - `services/hl-capture/src/backlog.rs`
  - `crates/storage-ports/src/archive.rs`
  - `crates/canonical-archive/src/raw.rs`
  - focused spool/archive tests
- Changes:
  - Separate local monotonically contiguous record sequence from opaque/native
    source cursor.
  - Preserve byte offsets, epoch identity, line hashes, parser disposition, and
    exact raw bytes.
  - Keep the existing block-height path byte-compatible and add a distinct
    cursor-policy constructor rather than relaxing all continuity checks.
- Verification:
  - RED tests for non-unit byte offsets, restart overlap, epoch rotation,
    partial-line exclusion, exact duplicates, conflicting duplicates, range
    proof, corruption, and legacy manifest compatibility.
  - Focused storage and capture tests.
- Expected result: raw spool/archive replay is lossless and deterministic for
  both height and byte-offset sources.

### M3. Wire auxiliary acquisition and observable health

- Goal: make configured Node V1 auxiliary streams active in `hl-capture run`.
- Files / systems:
  - `services/hl-capture/src/config.rs`
  - `services/hl-capture/src/service.rs`
  - `services/hl-capture/src/source_runtime.rs`
  - `services/hl-capture/src/metrics.rs`
  - `services/hl-capture/src/status.rs`
  - runtime integration tests
- Changes:
  - Start one bounded acquisition/drain task per enabled `NodeLine` source.
  - Spool and raw-archive before ACK; persist parse quarantine before ACK.
  - Expose source lag, spool depth, current epoch/cursor, partial-line state,
    last durable observation, qualification state, and latched failure.
  - Use cancellation-safe shutdown and bounded backpressure.
- Verification:
  - RED runtime tests proving the previously inert configured source advances.
  - Rotation, partial-write, downstream outage, capacity, shutdown, and restart
    integration tests.
- Expected result: both source halves are durably captured and observable, but
  no canonical trade is emitted before the join milestones.

### M4. Capture the byte-first operator corpus gate

- Goal: obtain the immutable same-build evidence needed to design the committed
  parser and zero-trade completion contract without guessing.
- Files / systems:
  - private operator recording directory outside Git
  - `docs/runbooks/operator-node-recording.md`
  - reviewed `NodeSourceQualificationManifestV1`
  - derived redistributable fixtures only when explicitly permitted
- Changes:
  - Record simultaneous non-empty `replica_cmds` and block-batched trade output
    from one exact verified node build and retained command line.
  - Cover active trades, empty blocks, multiple matches, non-trade interleave,
    node restart overlap, rotation, exact duplicates, gaps, catalog changes,
    schema drift, and maximum observed record size.
  - Prove or reject the auxiliary transaction-hash relation, complete committed
    match projection, per-row `start_pos` semantics, exact block-time
    normalization, and source-native evidence for an empty/missing trade batch.
  - Verify every byte hash and assign redistribution class before copying any
    derivative into the repository.
- Verification:
  - Independent manifest, signature/build/hash, privacy, and redistribution
    review.
  - Replay raw capture from M3 and require byte-identical manifest/file digests.
- Expected result: an approved, private byte-first corpus supports one exact
  source profile. Until this external gate passes, M5 and later joined-trade
  milestones are HOLD and all deployed/live/source qualifications remain false.

### M5. Parse the qualified committed execution projection

- Goal: derive transaction and match order from retained committed evidence,
  not auxiliary row order.
- Files / systems:
  - `crates/hl-protocol/src/node/v1.rs`
  - `crates/canonical-events/src/node_mapping.rs`
  - private operator corpus and derived redistributable fixtures when permitted
  - parser golden/fuzz/boundary tests
- Changes:
  - Add only the exact `actions-and-responses` variants proven by an approved
    manifest/build recording.
  - Produce ordered transaction identities and complete ordered match
    commitments, including interspersed non-trade transactions.
  - Reject unknown/changed shapes and any response that cannot prove complete
    trade projection.
- Verification:
  - The approved M4 recording must exist before implementation.
  - RED tests are derived byte-first from that recording and include empty,
    multi-transaction, multi-match, repeated-looking rows, maximum-size,
    unknown-field, and schema-drift cases.
- Expected result: exact qualified recordings parse into a deterministic
  committed execution descriptor; unproven builds/shapes remain blocked.

### M6. Freeze and implement the pure fail-closed committed trade join

- Goal: construct committed trades only when both projections are exactly
  equal.
- Files / systems:
  - `crates/canonical-events/src/node_mapping.rs`
  - `crates/canonical-events/src/block.rs`
  - Create `docs/contracts/committed-trade-join-v1.md`
  - focused join/property tests
- Changes:
  - Freeze transaction-manifest, complete ordered match-commitment,
    source-native completion, join-proof, stable failure-precedence, and
    deterministic lifecycle-time semantics from the approved M4 corpus.
  - Implement `CommittedTradeJoinV1` as a pure function over immutable inputs.
  - Require exact chain/instance/height/time, transaction order, complete match
    order/count/fields, catalog, profile, and raw hash equality to the proof.
  - Derive canonical transaction/full event indices from committed execution.
  - Construct new committed events with fixed two-source evidence ordering and
    a multi-source block proof.
  - Add `BlockCandidate::try_new_joined` (or equivalent proof-bearing
    constructor) without relaxing `try_new`.
- Verification:
  - RED mutation matrix perturbs every compared field independently.
  - Tests cover non-trade transactions between trades, multi-match, empty
    block, identical-looking legitimate rows, duplicates, both source roles,
    known-at policy, and unproved multi-source rejection.
- Expected result: only complete exact joins yield a canonical committed block;
  all failures are atomic and stable.

### M7. Add durable pending joins and two-phase receipts

- Goal: retain deterministic join state across either source arriving first and
  every archive/finalization crash boundary.
- Files / systems:
  - Create `services/hl-capture/src/join/`
  - Extend quarantine and status schemas
  - Add focused durable-store tests
- Changes:
  - Index candidates by chain, node instance, and height while preserving exact
    source identities/cursors/hashes.
  - Implement `AwaitMoreEvidence`, source-native missing, ambiguity,
    divergence, quarantine, and deterministic recovery.
  - Write a create-once `JoinDecisionV1` before archive; after an idempotent
    archive append, write a create-once `JoinedBlockFinalizationV1` bound to the
    archive identity fields. Neither record contains an unavailable future
    receipt.
  - On restart, recompute and verify decision bytes. If archive exists without
    finalization, verify its logical identity and finish finalization. If
    finalization exists, require its decision/archive bindings to match.
  - Treat a wall-clock lag threshold as health only; never use it to decide
    replay truth.
- Verification:
  - RED tests for either arrival order, exact replay, restart overlap,
    conflicting/multiple candidates, missing completion, decision mismatch,
    archive-without-finalization, finalization mismatch, and crash after every
    durable write.
- Expected result: pending and terminal join decisions reproduce byte-for-byte
  after restart without advancing the canonical cursor, and archive recovery
  has no dependency cycle.

### M8. Integrate join-before-sequence runtime and restart recovery

- Goal: prevent empty or partial committed blocks from reaching the existing
  coordinator before their source contract is complete.
- Files / systems:
  - `services/hl-capture/src/committed_pipeline.rs`
  - `services/hl-capture/src/source_runtime.rs`
  - `services/hl-capture/src/sequencer/`
  - `services/hl-capture/src/coordinator.rs`
  - runtime E2E harness
- Changes:
  - Route both durable backlogs through the join store before sequencing.
  - Persist and verify the join decision, canonical archive, and archive-bound
    finalization before advancing source/canonical cursors.
  - Hold later heights behind an unresolved earlier committed height.
  - Republish safely after restart and prohibit late amendment.
- Verification:
  - Crash failpoints after each source fsync, raw archive, join decision,
    canonical archive, finalization, publication plan, publish/ACK, and cursor
    advancement.
  - Compare uninterrupted/restarted event IDs, payload hashes, block hashes,
    cross-run archive semantic projections, and cursor state. That cross-run
    projection includes canonical block hash, object SHA-256, schema
    fingerprint, event IDs/payload hashes, counts, and source hashes; it
    excludes `durable_at` and raw manifest SHA-256 because the current manifest
    embeds its operational creation time. Within one archive, finalization must
    still bind and verify the actual manifest SHA-256. A future normalized
    semantic-manifest digest may be compared only if its schema explicitly
    omits operational times.
  - Run the same canonical input into two fresh archive roots with deliberately
    different archive clocks and require equal cross-run semantic projections
    while allowing different raw manifest hashes.
- Expected result: one deterministic exactly-once canonical history under
  source lag, process crash, and restart.

### M9. Make `hl-core` a durable canonical-state consumer

- Goal: turn the already-tested state-apply primitive into a runnable service.
- Files / systems:
  - `services/hl-core/src/main.rs`
  - `services/hl-core/src/lib.rs`
  - service config/status/metrics modules
  - integration and restart tests
- Changes:
  - Consume committed block markers/events, verify archive/publication
    evidence, restore the canonical reducer/checkpoint, call
    `apply_block_durably`, and ACK only after receipt verification.
  - Expose lag, last block/state hash, checkpoint/rebuild status, quarantine,
    health, shutdown, and kill-switch state.
- Verification:
  - RED integration tests for clean start, restart, duplicate delivery,
    out-of-order/missing event, archive mismatch, storage failure, and graceful
    shutdown.
  - End-to-end capture to core replay yields the same state hash as offline
    replay.
- Expected result: the read-only product has a runnable durable source-to-state
  path, not only library-level reducers.

### M10. Retain runtime, soak, replay, and qualification evidence

- Goal: make the source path operable for long-running evidence collection.
- Files / systems:
  - `tools/ci/` or a dedicated evidence runner
  - `docs/runbooks/`
  - report JSON schema and retained evidence directory
  - local product status/API consumers
- Changes:
  - Add bounded and long-running modes with machine-readable progress,
    heartbeat, resource usage, source lag, quarantine, restart count, and final
    invariant summary.
  - Compare independent offline reprocessing against live outputs.
  - Derive qualification booleans from artifacts and registry state; never
    accept them as CLI inputs.
- Verification:
  - Focused report-oracle mutation tests.
  - Short deterministic E2E gate plus operator-selected long soak.
  - `just generated`, `just deny`, `just oss-audit`, focused packages,
    serialized `RUST_TEST_THREADS=1 just verify`, and `git diff --check`.
- Expected result: retained evidence distinguishes `repo-ready`,
  `runtime-proven`, and every still-blocked higher gate.

## Verification

- Focused milestone commands are recorded with their RED and GREEN outputs in
  the Progress Log and scoped commit message/trailer where appropriate.
- Full local gate after each vertical runtime milestone:
  - `just generated`
  - `just deny`
  - `just oss-audit`
  - `cargo +1.97.1 test --workspace --all-targets --all-features --locked --offline`
  - `cargo +1.97.1 clippy --workspace --all-targets --all-features --locked --offline -- -D warnings`
  - `cargo +1.97.1 fmt --all -- --check`
  - `RUST_TEST_THREADS=1 just verify`
  - `git diff --check`
- Manual smoke:
  - Start capture with a synthetic qualified-*false* dual-stream fixture and
    verify raw durability while canonical trade publication remains blocked.
  - Start with an approved private recording and verify exact joined committed
    trades, restart identity, state application, health, and clean shutdown.
  - Perturb one copied input byte and verify quarantine plus unchanged
    canonical/state cursors.
- Long-running evidence:
  - Run with bounded disk/retention limits, rotate both node outputs, force at
    least one restart and downstream outage, and retain report/log/archive
    digests. A soak duration is reported as evidence, never translated into a
    higher qualification by itself.

## Decision Log

- 2026-08-03: Keep auxiliary observations in the reconciliation lane and
  construct a new committed event only from a validated join proof. Mutating
  the provisional event would erase the evidence boundary.
- 2026-08-03: Require full committed match projection equality. Transaction
  hash membership, timestamp proximity, and ordered-subsequence checks cannot
  prove complete execution order or absence of missing/extra rows.
- 2026-08-03: Use distinct source IDs under one stable node instance ID. The
  two outputs are different immutable records but are not independent sources.
- 2026-08-03: Separate local spool sequence from native source cursor. This
  preserves byte offsets and rotation epochs without weakening the existing
  height-continuity invariant.
- 2026-08-03: Missing evidence becomes a replay disposition only through a
  source-native completion/advancement marker. Wall-clock time affects health,
  not canonical truth.
- 2026-08-03: Defer exact committed action/response decoding until a retained
  same-build recording proves the schema and trade-hash relation. Guessing the
  wire shape would be an unsafe source-of-truth invention.
- 2026-08-03: Wire capture and durable join infrastructure before the exact
  production parser. Those components are independently useful and can be
  tested fail-closed while the qualification registry remains empty.
- 2026-08-03: Split persistence into a pre-archive join decision and a
  post-archive finalization receipt. A single receipt cannot contain archive
  identity before the archive operation creates it.
- 2026-08-03: Canonical event lifecycle timestamps are pure projections of the
  persisted source receive times. Runtime processing/durability clocks are
  operational telemetry and are excluded from canonical bytes.
- 2026-08-03: Compare archive identity hashes across restart, not the full
  `ArchiveReceipt`; its `durable_at` field and the manifest hash derived from a
  manifest containing that time are intentionally operational across roots.
  The actual manifest hash remains mandatory for finalization within one
  archive.

## Progress Log

- 2026-08-03: Completed three independent read-only audits of the code path,
  contract/spec boundary, and retained fixtures/evidence at clean HEAD
  `1de17ce8a2f649682b2404f68f2f9e7a591a7173`.
- 2026-08-03: Confirmed the runtime starts only committed block-directory
  sources; node-line sources are valid in configuration but inert.
- 2026-08-03: Confirmed all retained source/capture/position evidence is
  synthetic or unassessed and no production recording exists in the checkout.
- 2026-08-03: Current qualification is `repo-ready` for prior synthetic
  reducer/replay work only; the production source/commit join is not yet
  `runtime-proven`.
- 2026-08-03: Independent design review returned HOLD on an impossible receipt
  dependency, premature join-contract freeze, inverted corpus order,
  nondeterministic lifecycle-time policy, and an overbroad M1 qualification
  claim. The plan now splits decision/finalization, moves the corpus gate to
  M4, defers join semantics to M6, freezes replay-stable times, and limits M1 to
  sealed qualification/recording envelopes.
- 2026-08-03: Independent re-review returned GO after the cross-run archive
  projection excluded the operational-time-derived raw manifest hash and added
  a two-root/different-clock regression. The actual manifest hash remains bound
  within one archive's finalization record.
- 2026-08-03: Committed M1 at `5fdf6d65af51a778dc5cd37393b718daab719e88`
  (`feat(protocol): add sealed node source qualification`). The first focused
  test run failed RED with unresolved qualification types; the final focused
  qualification suite passed 14/14 and the full `hl-protocol` package passed
  34 tests plus doc tests.
- 2026-08-03: Requirements review initially held M1 for an unbound
  time-normalization artifact and acceptance of the overriding `--write-fills`
  flag. Quality review initially held it for conflated block-height/byte-offset
  cursor evidence, incomplete read-only inspection/generation documentation,
  and a vacuous registry-mutation test. All five findings were corrected and
  both independent re-reviews returned GO.
- 2026-08-03: M1 strict package Clippy, formatting, and `git diff --check`
  passed. `just generated` passed against detached commit `5fdf6d6` with
  `generated-check:ok`; `just deny` passed advisories, bans, licenses, and
  sources; `just oss-audit` passed with 527 files. The production qualification
  registry intentionally remains empty, so this is `repo-ready` evidence only.
- 2026-08-03: Next: implement M2 with RED tests while preserving the legacy
  height-contiguous raw manifest bytes and introducing an explicit byte-offset
  cursor policy.

## Rollback / Recovery

- If a milestone fails: stop its runtime path behind the qualification gate,
  retain both raw spools/quarantine evidence, and leave the canonical cursor at
  the last verified joined block.
- Safe fallback: continue read-only raw capture and offline inspection with
  qualification booleans false. Do not publish an empty committed block for a
  height that may later receive auxiliary trade evidence.
- Compatibility: introduce versioned types, manifests, constructors, and
  reason codes. Preserve existing synthetic fixtures, single-source candidate
  behavior, committed empty-block test path, archive formats, and V1 reducers
  unless an explicit migration with byte-compatibility tests is part of the
  reviewed milestone.
- Data recovery: rebuild pending joins and canonical state from verified raw
  archives, join decisions, archive identities, and finalization receipts. An
  archive-without-finalization is recovered only by exact idempotent archive
  verification followed by create-once finalization. Never repair by editing
  hashes, skipping a gap, deleting a quarantine record, or accepting a newer
  profile for older bytes.
