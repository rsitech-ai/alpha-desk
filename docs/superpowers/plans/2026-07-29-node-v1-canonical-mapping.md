# Node V1 Canonical Mapping and Upcaster Implementation Plan

> **For agentic workers:** Follow the repository TDD contract. Each behavior
> milestone starts with a focused failing test, then the smallest production
> implementation, then focused and workspace verification.

## Goal

- User-visible outcome: Alpha Desk can deterministically convert every
  source-record shape represented by its pinned public Node V1 corpus into
  validated canonical events or an explicit evidence-only outcome, and can
  reproduce the same hashes from a command-line inspection tool.
- How to see it working: run `canonical-inspect` over the committed batched
  Node V1 fixtures and obtain the committed expected event/block manifest;
  rerunning from a clean detached tree produces byte-identical output.

## Current State

- Relevant paths:
  - `crates/hl-protocol/src/node/v1.rs`
  - `fixtures/source/node-v1/`
  - `crates/canonical-events/src/{event_id,input,block}.rs`
  - `schemas/proto/canonical/v1/events.proto`
  - `docs/superpowers/plans/2026-07-24-02-truth-layer.md`
- Existing behavior:
  - The source boundary recognizes transaction blocks, fills, order statuses,
    raw book diffs, miscellaneous ledger events, and market metadata while
    preserving exact bytes.
  - Stable event identity, validated production event construction, and
    deterministic block envelopes are implemented at `3f42fd9`.
  - The canonical payload schema covers the complete design event-kind set,
    but most in-memory payloads are still opaque and no source mapper,
    upcaster registry, or `canonical-inspect` tool exists.
- Constraints:
  - The fixture corpus contains normalized public documentation examples, not
    byte-exact production node recordings.
  - Current upstream node documentation additionally advertises HIP-3 oracle
    updates and system/CoreWriter action streams, but the scoped corpus has no
    immutable schema examples for them. They must not be guessed.
  - Auxiliary fills/order/book records do not always expose a block height or
    transaction hash unless the node uses `--batch-by-block` or
    `--stream-with-block-info`. Canonical committed mapping therefore requires
    block-bearing envelopes and fails closed without them.
  - No trading, signing, private keys, order submission, or custody capability
    is in scope.

## Target State

- Desired behavior:
  - Source evidence records an explicit stable source-event sub-index without
    losing the parent source offset and content hash.
  - A pure Node V1 mapper accepts only validated block-bearing observations,
    a versioned market catalog, and explicit lifecycle/source metadata.
  - Mapping order follows source array order. Each accepted mapped
    discriminant produces the exact canonical event family required by its
    documented semantics.
  - Empty transaction blocks and accepted evidence-only records return an
    explicit typed disposition instead of being silently dropped.
  - Unknown variants, missing block/transaction identity, unmapped markets,
    precision failures, and ambiguous semantics return stable typed errors
    containing the fixture/source offset and byte location when available.
  - The semantic upcaster accepts the current canonical major/minor range,
    validates the payload, and rejects unsupported historical/future versions.
    Raw evidence is never rewritten.
  - `canonical-inspect` emits a deterministic manifest with source hashes,
    event IDs, payload hashes, and block hashes.
- Non-goals:
  - Claiming real-node or cross-operator qualification from documentation
    fixtures.
  - Inventing proprietary operator-feed, independent-source, HIP-3 oracle, or
    CoreWriter schemas.
  - Using public REST/WebSocket payloads as substitutes for committed source
    contracts.
  - Implementing continuity, persistence, archive publication, or a daemon in
    this milestone.

## Risks and Failure Modes

- Mapping a user-fill record as a globally complete trade can lose the maker
  side or duplicate the same trade across user streams.
- A source-derived fallback transaction identity can collide or change across
  sources unless its domain and inputs are frozen.
- Treating raw book diffs or snapshots as committed canonical history can
  fabricate continuity.
- Market symbols such as `@107` and HIP-3 names are ambiguous without a
  versioned catalog.
- Adding a source sub-index to the wire schema can accidentally change V1
  compatibility or hash projections.
- A CLI that supplies fabricated block context would produce attractive but
  operationally meaningless hashes.

## Milestones

### M1. Freeze evidence sub-index and mapping dispositions

- Goal: represent one-to-many mapping and intentional evidence-only outcomes
  without encoding sub-indexes into source cursor strings.
- Files / systems:
  - `schemas/proto/canonical/v1/events.proto`
  - `crates/api-contracts/src/lib.rs`
  - `crates/canonical-events/src/lib.rs`
  - focused round-trip and schema-compatibility tests
- Changes:
  - Add optional/presence-aware `source_event_index` to wire source evidence.
  - Add a validated accessor and preserve it through canonical envelope
    encoding/decoding.
  - Define closed `MappingDisposition` values for mapped events, empty block,
    and evidence-only auxiliary input.
- Verification:
  - focused canonical envelope tests fail before implementation and pass after.
  - schema compatibility remains additive.
- Expected result: downstream code can distinguish a deliberate zero-event
  outcome from a parser omission and can trace each mapped event to its parent
  evidence and stable sub-index.

### M2. Map block-bearing public Node V1 fixtures

- Goal: implement the public source mappings supported by immutable local
  fixtures and explicit block context.
- Files / systems:
  - `crates/canonical-events/src/parser.rs`
  - domain payload modules under `crates/canonical-events/src/payloads/`
  - `crates/canonical-events/tests/golden_blocks.rs`
  - batched fixtures and provenance manifest under `fixtures/source/node-v1/`
- Changes:
  - Parse fixed-point strings with existing checked domain types.
  - Resolve market symbols only through a versioned explicit catalog.
  - Map complete trade records, order lifecycle statuses, order-book lifecycle
    changes, ledger transfers, liquidation records, and metadata records where
    the documented fixture contains sufficient semantics.
  - Return evidence-only or a typed ambiguity error where a record is not a
    complete canonical fact; never manufacture missing counterparties,
    transaction order, or balances.
  - Derive fallback source transaction identities only for explicitly
    auxiliary records, using a documented domain-separated projection over
    chain, block, source stream, parent content hash, and source-event index.
- Verification:
  - golden mapping tests cover every fixture manifest discriminant.
  - mutation tests cover unknown status, missing block metadata, unmapped
    market, over-precision value, and reordered batched events.
- Expected result: every locally accepted corpus record has one explicit,
  test-proven mapping disposition and deterministic provenance.

### M3. Add the semantic-version upcaster boundary

- Goal: make canonical payload-version handling explicit before persisted
  historical records exist.
- Files / systems:
  - `crates/canonical-events/src/upcast.rs`
  - `crates/canonical-events/tests/upcast.rs`
- Changes:
  - Parse and validate semantic versions.
  - Accept current V1 payloads through a validating identity upcast.
  - Reject unsupported older/future major versions with stable reason codes.
  - Define the registry shape required for future pure stepwise upcasters
    without claiming nonexistent historical versions.
- Verification:
  - focused tests prove current acceptance, raw-evidence immutability, malformed
    version rejection, and unsupported-version rejection.
- Expected result: persisted readers have one fail-closed version boundary and
  future migrations cannot silently rewrite source evidence.

### M4. Add deterministic canonical inspection

- Goal: produce operator-reviewable mapping and hash evidence without starting
  a service.
- Files / systems:
  - `tools/canonical-inspect/`
  - workspace `Cargo.toml`
  - deterministic expected manifest under `fixtures/canonical/node-v1/`
  - `docs/DEVELOPMENT.md`
- Changes:
  - Read only block-bearing committed fixtures plus their provenance/catalog.
  - Emit a stable, sorted JSON manifest through create-new/atomic output
    semantics.
  - Include fixture identity, source hash, mapping disposition, event IDs,
    payload hashes, and canonical block hash.
  - Refuse production-readiness language and fail on unqualified/unmapped
    corpus entries.
- Verification:
  - CLI integration tests cover success, output collision, malformed fixture,
    and missing catalog/block metadata.
  - generated check reproduces the committed manifest in a detached tree.
- Expected result: the documented Task 5 inspection path is runnable and
  deterministic, while corpus qualification remains visibly separate.

### M5. Full verification and evidence ledger

- Goal: land one reviewable canonical-mapping milestone with claims no broader
  than its evidence.
- Files / systems:
  - `README.md`, `CHANGELOG.md`, `docs/STATUS.md`, `docs/ROADMAP.md`
  - `tools/ci/check-generated.sh`
- Changes:
  - Record implemented mappings and exact corpus limits.
  - Record current upstream stream gaps: HIP-3 oracle and system/CoreWriter
    schemas/recordings remain required.
  - Add deterministic mapping/CLI checks to the detached-tree gate.
- Verification:
  - `cargo +1.97.1 fmt --all -- --check`
  - focused Rust tests and Clippy with warnings denied
  - `just verify`
  - `just generated`
  - `just oss-audit`
  - review exact diff and commit only intentional files
- Expected result: the branch is clean and reproducible; Task 5 is either
  complete for the qualified corpus or its remaining exact corpus blockers are
  stated without a broad production claim.

## Verification

- `cargo +1.97.1 test -p hl-protocol --test node_golden --locked --offline`
- `cargo +1.97.1 test -p canonical-events --all-targets --locked --offline`
- `cargo +1.97.1 test -p canonical-inspect --locked --offline`
- `cargo +1.97.1 clippy -p hl-protocol -p canonical-events -p canonical-inspect --all-targets --locked --offline -- -D warnings`
- `cargo +1.97.1 run -p architecture-check --locked --offline -- check`
- `just verify`
- `just generated`
- `just oss-audit`
- Manual smoke: run `canonical-inspect` twice from clean directories and compare
  output bytes and SHA-256.

## Decision Log

- 2026-07-29: Treat current Hyperliquid node README and L1 schema
  documentation as discovery inputs, but commit normalized fixtures with hashes
  before allowing them to become executable mapping contracts.
- 2026-07-29: Require block-bearing node output for canonical mapping.
  Standalone auxiliary lines are evidence, not permission to fabricate a block
  height or protocol order.
- 2026-07-29: Do not add HIP-3 oracle or system/CoreWriter parsers from flag
  names alone. Their immutable schemas and representative recordings are
  required first.
- 2026-07-29: Do not equate a user-fill API-shaped record with a complete
  two-sided trade unless the source contract supplies sufficient global
  identity/counterparty semantics.
- 2026-07-29: A first implementation attempt mapped a block-batched
  `--write-fills` record directly to `TradeMatched`. Review rejected and
  removed it before commit because the record exposes only one order and does
  not prove complete maker/taker semantics. Pin and qualify the distinct
  `--write-trades` contract or committed transaction-block mapping instead.

## Progress Log

- 2026-07-29: M11 canonical event/block identity completed and verified at
  `3f42fd9`.
- 2026-07-29: Audited current upstream public node documentation. Existing
  order-status coverage includes the currently documented rejection variants;
  upstream now advertises additional HIP-3 oracle and system/CoreWriter output
  streams absent from the pinned corpus.
- 2026-07-29: Implemented the M1 presence-aware source-event sub-index through
  the Protobuf, wire, and canonical domain boundaries. The focused round-trip
  test and additive schema compatibility check pass.
- 2026-07-29: Next: pin the complete public trade/transaction mapping contract
  and implement mapping dispositions without promoting incomplete fill
  evidence.
- 2026-07-29: Pinned the complete public `--write-trades` example inside the
  documented block wrapper, added it to the hashed non-production corpus, and
  implemented the conservative trade mapper. Market resolution is catalog-only;
  buyer/seller order is preserved; maker/taker IDs are intentionally absent;
  standalone and unsupported records have explicit evidence-only
  dispositions; mapped auxiliary trades remain `ProvisionalSource`.

## Rollback / Recovery

- If this fails: keep the isolated branch and failing fixtures/tests; do not
  weaken parsing or convert ambiguity into defaults.
- Safe fallback: revert only the milestone commit before any persisted V1
  record uses the added evidence field. After persisted adoption, add a new
  schema version instead of silently changing V1.
- External corpus blocker: preserve the public adapter port and report the
  missing immutable schema/recording; do not substitute public REST/WebSocket
  data or invented JSON.
