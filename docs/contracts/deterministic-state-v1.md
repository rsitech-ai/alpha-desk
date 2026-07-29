# Deterministic State V1

This document freezes the first storage-neutral state reconstruction contract.
It is an implemented domain foundation, not evidence that Stage 2 or any
action-bearing Hyperliquid state is complete.

## Authority boundary

`canonical-ledger` accepts only canonical blocks classified
`CommittedPrimary` or `CommittedIndependent`. It does not accept provisional,
expired, reconciled-snapshot, or correction blocks. Correction semantics remain
unimplemented and default-deny.

Every non-empty block event must be owned by the configured reducer set for its
exact `EventKind` and semantic schema version. An unsupported event returns
`ledger.unsupported_event` before the durable state can advance. Opaque payload
bytes and envelope routing metadata are never interpreted as account, order,
market, fee, funding, transfer, or position semantics.

An actually empty committed block is a supported watermark-only transition.
The production `WatermarkOnlyReducerV1` freezes that boundary under reducer-set
version `hyperliquid-alpha-desk-watermark-only@1.0.0`; its qualified
action-bearing event registry is deliberately empty.

`CanonicalTradeReducerV1` is a narrower canonical-semantic reducer under
version `hyperliquid-alpha-desk-canonical-trade@1.0.0`. It owns only
`TradeMatched` schema `1.0.0` and requires:

- one payload market that exactly matches the sole envelope market;
- a stable trade identity;
- exactly two distinct observed participant addresses; and
- strictly positive fixed-point price and quantity.

It stores one immutable trade fact, two ordinal participant legs carrying the
same exact quantity, and one passed `trade-quantity-symmetry@1.0.0`
reconciliation record. Participant ordinals preserve canonical evidence order;
they are not buyer/seller or maker/taker labels. The reducer does not infer
position direction, order effects, fees, PnL, funding, transfer, margin, or
liquidation semantics. A duplicate trade identity or malformed contract
rejects the complete candidate block.

Each trade-state value is bounded to 16 KiB, encoded as strict field-ordered
JSON with a versioned schema, and accepted on restore or inspection only when
decode and canonical re-encode reproduce the exact bytes. State keys bind the
trade identity and participant ordinal; typed `decode_at` helpers reject a
record presented under another key.

`CanonicalOrderReducerV1` freezes the exact V1 order lifecycle under reducer-set
version `hyperliquid-alpha-desk-canonical-order@1.0.0`. It owns only exact
schema `1.0.0` events for `OrderAccepted`, `OrderRested`, `OrderModified`,
`OrderPartiallyFilled`, `OrderFilled`, `OrderCancelled`, and `OrderRejected`.
Every action-bearing order event requires one envelope market and one account
that exactly match the payload or current state. A rejection requires no market
and exactly its payload account.

The reducer stores:

- an immutable `order-fact.v1` record for every owned event;
- one key-bound `order-current.v1` record for each accepted order; and
- an immutable `order-transition.v1` assessment binding the event and payload
  hash, prior current-record hash, result current-record hash, rule version, and
  applied or recorded-rejection status.

Order and transition keys frame the exact market, order, and event identities.
Rejection keys instead frame the account, client-order, and event identities.
All three record families are bounded to 16 KiB, use strict field-ordered JSON,
deny unknown fields, and require byte-exact canonical re-encoding plus
key-bound decoding.

The lifecycle table defaults to denial. Resting is allowed only after
acceptance or modification; modification is allowed only before terminal state;
partial and terminal fills use checked fixed-point arithmetic; a partial fill
must leave a positive remainder; a terminal fill must consume the exact
remainder; and cancellation must report the exact current remainder. Filled or
cancelled orders never transition again. Modification updates the active
accepted quantity to exact filled plus new remaining quantity. A rejection
creates a fact and assessment but never an active order.

This reducer does not infer maker/taker, venue execution role, position,
balance, fee, funding, margin, liquidation, or order-book state. Its inputs are
canonical-event contracts, not evidence that any deployed source emits those
contracts correctly.

`CanonicalMarketReducerV1` freezes the point-in-time V1 market registry under
reducer-set version `hyperliquid-alpha-desk-canonical-market@1.0.0`. It owns
only exact schema `1.0.0` events for `DexCreated`, `AssetContextUpdated`,
`MarketCreated`, `MarketMetadataChanged`, `MarketHalted`, `MarketResumed`,
`OpenInterestCapChanged`, `MarginTableChanged`, `OracleUpdated`,
`FundingRateUpdated`, `OutcomeCreated`, and `OutcomeResolved`.

The reducer stores one immutable `market-fact.v1` record for every owned event
and key-bound current records for DEX, asset context, market, and outcome
identity. DEX, asset, market, metadata-version, event, and outcome identities
cannot be reused. A market can be created only after its DEX and two distinct
assets exist in candidate state. Creation installs the exact metadata interval
`creation@1.0.0`; tick size, lot size, price scale, and quantity scale come
only from the canonical creation values.

Metadata intervals are key-bound by framed market and version identities and
cannot overlap. A later hash-only `MarketMetadataChanged` must occur at a
strictly later block with a lexically increasing, unused version. It closes the
prior open interval at the previous block whether that interval is exact or
unresolved, then opens a new unresolved interval. Consecutive hash-only
versions therefore remain point-in-time complete without inventing exact
metadata.

An unresolved transition removes exact tick, lot, scale, cap, margin-table,
oracle, and funding applicability from current state. Public scale getters
return `Option<u32>`, and all value-dependent getters return absence while
metadata is unresolved. Open-interest cap, margin-table, oracle, funding, and
outcome resolution events fail with `market_state.metadata_unresolved`; the
reducer does not copy values across an unproven metadata hash. Immutable facts
retain the prior events without presenting their values as current.

Status changes default-deny: only active-to-halted and halted-to-active are
valid. Outcomes have immutable market-bound identity and may resolve exactly
once. Oracle and funding effective times never regress. Open-interest cap and
margin-table changes compare their asserted previous values with current state
after the first such event establishes the predecessor omitted by
`MarketCreated`; every later mismatch rejects the complete candidate block.
Envelope market and account lists must exactly match payload identity for every
event.

All market-registry record families are bounded to 16 KiB, encoded as strict
field-ordered JSON with unknown fields denied, and accepted only when canonical
re-encoding reproduces the exact bytes. Keys use length framing and typed
`decode_at` helpers reject records presented under another identity. Oversized
identifiers fail key construction instead of panicking. Key builders compute
the complete framed size against the 64 KiB ceiling before allocation and use
fallible exact reservation before copying identities.

This registry is a storage-neutral canonical prerequisite. It does not qualify
deployed source mapping, authoritative metadata snapshots, external oracle
reconciliation, account or position effects, margin formulas, order books, or
Stage 2 readiness.

## Block atomicity

The ledger:

1. verifies the frozen reducer-set version, chain, confirmation class, duplicate
   disposition, and next height;
2. clones the current pure state into an isolated candidate;
3. applies events in canonical block order;
4. enforces per-event and per-block mutation bounds;
5. runs block-wide reducer invariants;
6. hashes the complete candidate state; and
7. returns an opaque prepared transition without changing visible state; and
8. swaps that candidate into the visible ledger only after an explicit commit.

A reducer error, unsupported event, invalid deletion, duplicate mutation key,
limit violation, or invariant failure discards the candidate. The prior state
bytes, state hash, and checkpoint remain unchanged.

An exact redelivery of the latest block returns `AlreadyApplied` with the
existing checkpoint. The same height with a different canonical block hash
returns `ledger.canonical_divergence`.

### Durable commit handoff

`storage-ports::AtomicStateCommit` binds one prepared `StateDelta` to its exact
complete `StateImage`. Construction verifies the chain, block height, canonical
block hash, reducer-set version, and after-state hash. The request exposes the
before-state hash so an adapter can reject a store that is ahead, behind, or on
a divergent transition.

`hl-core::apply_block_durably` enforces the visibility order:

1. prepare the domain transition without changing the ledger;
2. validate the storage-neutral atomic commit request;
3. ask the state adapter to persist all mutations and the block checkpoint as
   one operation;
4. validate that the adapter receipt names the exact height, canonical block
   hash, and state hash; and
5. commit the already-durable prepared transition to the visible ledger.

A storage error or mismatched receipt leaves the in-memory ledger unchanged.
The port requires restart loading to return only the latest complete state
image and to reject partial, corrupt, or oversized state. It also gives stable
errors for lock contention, corruption, conflicting history, resource limits,
and I/O failure. Vendor types do not enter the ledger, replay, or service
contracts.

This freezes the atomicity seam; it does not implement the production store.
The exact RocksDB 11.1.x adapter, column-family mapping, WAL/compaction policy,
lock ownership, crash recovery, and corruption qualification remain required.

## State key and mutation rules

State keys contain:

- a namespace of 1–96 ASCII bytes;
- a lowercase ASCII letter as the first byte;
- an ASCII alphanumeric final byte;
- only lowercase ASCII letters, digits, `.`, `-`, or `_`; and
- a non-empty opaque key of no more than 64 KiB before tighter runtime limits.

One event cannot emit multiple mutations for the same key. A delete of a
missing key is invalid instead of silently becoming a no-op.

Production defaults currently bound:

- events per block: `100000`;
- mutations per event: `4096`;
- encoded key bytes: `4096`;
- value bytes: `16777216`; and
- aggregate block delta bytes: `268435456`.

These are safety ceilings, not measured performance qualifications. Deployment
profiles may tighten them but cannot set zero or make an individual key/value
limit larger than the block delta limit.

## Canonical state bytes

All integer fields use unsigned big-endian bytes. Every variable byte sequence
is prefixed by its unsigned 64-bit byte length. Entries use `BTreeMap` byte
order and therefore never depend on hash-map iteration or reducer mutation
emission order.

The V1 byte sequence is:

```text
frame("hyperliquid-alpha-desk/state-image/v1")
frame(chain_id)
u64(first_height)
frame(reducer_set_version)
u8(watermark_present)
if present:
  u64(block_height)
  bytes32(canonical_block_hash)
u64(entry_count)
for each sorted entry:
  frame(namespace)
  frame(key)
  frame(value)
```

The state hash is BLAKE3 derive-key hashing with context
`hyperliquid-alpha-desk/state-hash/v1` over the complete V1 state bytes.
Changing the state-image schema or hash framing requires a new explicit
version. A golden empty-range vector is checked in the canonical-ledger tests.

## Checkpoint identity and restore

The checkpoint contract binds:

- chain ID;
- applied block height;
- canonical block hash;
- deterministic state hash; and
- reducer-set version;
- archive manifest ID and SHA-256;
- canonical schema fingerprint;
- domain-separated state-image file hash and byte count; and
- a domain-separated content-derived `CheckpointId`.

The V1 JSON manifest has a fixed field order, denies unknown or duplicate
fields, uses lowercase 32-byte hexadecimal hashes, and must re-encode to the
exact input bytes. Whitespace, alternate field order, or other semantically
equivalent JSON is rejected as noncanonical.

Restore first decodes the bounded state image, then recomputes its state hash
and file hash, rebuilds the checkpoint identity, verifies the canonical
manifest bytes, and finally compares the complete chain/reducer/archive/schema
compatibility contract. A restored `CanonicalLedger` must use the exact reducer
set recorded in the image.

The RocksDB checkpoint adapter remains a later milestone. The current in-memory
state image is a deterministic reference representation, not a claim that a
multi-gigabyte production state should be materialized in one allocation.

### Local checkpoint store

`canonical-state-store` implements the synchronous `StateCheckpointStore`
port for the deterministic reference image. It retains an opened root
directory descriptor and performs generation, file, and rename operations
relative to that descriptor. Renaming or replacing the configured pathname
after open cannot retarget checkpoint I/O.

Each content-derived generation is:

```text
<private-root>/
  state-checkpoint-v1-<64 lowercase hex>/
    state.bin
    manifest.json
```

The root and generation directories are owner-only `0700`; files are
owner-only `0600`. Publication creates a random owner-only staging directory,
writes and fsyncs `state.bin`, writes and fsyncs `manifest.json` last, fsyncs
the staging directory, renames the complete directory with `NOREPLACE`, and
fsyncs the retained root. A crash before rename leaves no content-addressed
generation; it can leave a hidden `.staged-*` directory, which is ignored by
load and must be removed only by a future descriptor-relative recovery scanner.
Ordinary error paths clean their own staging directory. An existing identical
content-derived generation is idempotent; conflicting content is rejected.

Open and load use `NOFOLLOW`, reject non-private objects and path-bearing
checkpoint IDs, bound every read before allocation, require both files, decode
the domain artifact, compare the directory ID, and only then validate runtime
compatibility. This store is crash-safe local reference checkpoint evidence;
the RocksDB-native hot-state/checkpoint implementation remains M5.

## Serial immutable-manifest replay

`replay-engine` is the serial reference implementation. A replay request binds
the chain, inclusive height range, ordered immutable block-manifest IDs,
expected starting state hash, canonical dataset name, and expected schema
fingerprint. Production defaults reject requests above 10,000,000 blocks or
100,000 manifests.

Before mutating the ledger, the engine verifies every manifest and rejects:

- a manifest for another chain;
- a manifest ID/content mismatch;
- a gap, overlap, reordering, duplicate ID, or incomplete requested range;
- a schema fingerprint mismatch;
- a range/count overflow; or
- a starting state hash or height incompatible with the current ledger.

Replay never follows mutable `CURRENT` archive pointers. Each verified
manifest is read by its content-derived identity after preflight. Blocks are
then applied serially through the same block-atomic ledger path. Archive
content divergence stops replay, while an unsupported or invalid block returns
the exact quarantine height and reducer reason after preserving all previously
committed blocks and rejecting the failing block.

Cancellation is observed only between blocks. Completed and cancelled receipts
bind the status, chain, planned range, start/final state hashes, reducer-set
version, applied count, last committed block identity, and every manifest
identity/range in canonical bytes. The receipt hash uses BLAKE3 derive-key
context `hyperliquid-alpha-desk/replay-receipt-hash/v1`; a fixed completed
receipt vector protects the V1 framing.

For operator-selected archive ranges, `CanonicalArchive::plan_range` resolves
the current catalog once, verifies the selected object bounds, and returns an
ordered immutable manifest plan. `state-replay archive-e2e` validates contiguous
manifest boundaries, chain and schema identity before creating evidence output,
then repeats rebuild and checkpoint resume using only that frozen plan. It does
not mutate the source archive.

## Current evidence and limitations

Focused tests prove:

- deterministic state bytes and a fixed hash vector;
- contiguous empty committed blocks;
- identical output across independent ledger instances;
- independence from reducer mutation emission order;
- whole-block rollback after a late event or invariant failure;
- invisible prepare followed by explicit commit and stale-preparation
  rejection;
- durable-store-before-visibility ordering, including unchanged visible state
  on storage failure or receipt mismatch;
- exact duplicate idempotence and same-height divergence;
- chain, height, confirmation, reducer-version, and support gates; and
- mutation bounds and ambiguous key rejection; and
- the production watermark-only reducer accepting empty primary/independent
  committed blocks while quarantining a typed trade block without state
  effects; and
- exact state-image decode/resume, canonical checkpoint manifest round-trip,
  state tamper detection, and all bound compatibility identities;
- private descriptor-relative checkpoint publication/load, including symlink,
  permission, truncation, incomplete-generation, and parent-path-retarget
  rejection; and
- two byte-identical clean replays, checkpoint-equivalent resume, immutable
  manifest reads after `CURRENT` advances, preflight rejection without
  mutation, wrong-chain and wrong-start-state rejection, block-boundary
  cancellation, poison-block quarantine, and a fixed replay-receipt hash; and
- read-only operator-archive planning, repeated rebuild, exact-boundary
  checkpoint resume, explicit unqualified evidence, and unchanged archive
  inspection before/after; and
- exact canonical trade fact, two-leg, and stored symmetry records; malformed
  late-event whole-block rollback; duplicate trade-identity rejection; bounded
  canonical codec/key binding; and archive replay/checkpoint equivalence for a
  three-block synthetic trade sequence; and
- a bounded operator-visible synthetic trade runner proving repeated rebuild,
  decoded record cardinality, private checkpoint resume, malformed-trade
  reducer failure, unsupported-schema quarantine, and private evidence
  publication; and
- exact order acceptance, resting, modification, partial fill, terminal fill,
  cancellation, and rejection state; immutable fact and hash-linked transition
  records; strict identity/key/codec binding; checked overfill and remainder
  rejection; terminal-state non-resurrection; and whole-block rollback after a
  late invalid order transition; and
- a bounded operator-visible synthetic order runner proving repeated rebuild,
  decoded lifecycle cardinality, private checkpoint resume, late-overfill
  reducer failure, unsupported-schema quarantine, owner-only evidence
  permissions, and explicit false Stage 1/2, deployed/live source, position,
  margin, and execution qualification.

This proves stored canonical trade-fact reconciliation and exact synthetic
order-lifecycle contracts. It does not prove deployed action-bearing source
compatibility, buyer/seller or maker/taker roles, position or balance state,
external snapshot reconciliation, RocksDB durability, a production replay
service, or Stage 2 readiness. The retained local order report at
`target/evidence/state-replay-order/20260729T185537Z-82818/report.json` covers
20 generated blocks, four independent rebuilds, a checkpoint after block 8,
80 facts and transitions, 20 current orders, 10 filled orders, 10 cancelled
orders, and 10 fact-only rejections. Runnable replay evidence remains generated
canonical-event evidence with source qualification explicitly unassessed.
