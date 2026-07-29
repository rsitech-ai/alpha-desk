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

## Block atomicity

The ledger:

1. verifies the frozen reducer-set version, chain, confirmation class, duplicate
   disposition, and next height;
2. clones the current pure state into an isolated candidate;
3. applies events in canonical block order;
4. enforces per-event and per-block mutation bounds;
5. runs block-wide reducer invariants;
6. hashes the complete candidate state; and
7. swaps the candidate into the visible ledger only after every prior step
   succeeds.

A reducer error, unsupported event, invalid deletion, duplicate mutation key,
limit violation, or invariant failure discards the candidate. The prior state
bytes, state hash, and checkpoint remain unchanged.

An exact redelivery of the latest block returns `AlreadyApplied` with the
existing checkpoint. The same height with a different canonical block hash
returns `ledger.canonical_divergence`.

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

## Current evidence and limitations

Focused tests prove:

- deterministic state bytes and a fixed hash vector;
- contiguous empty committed blocks;
- identical output across independent ledger instances;
- independence from reducer mutation emission order;
- whole-block rollback after a late event or invariant failure;
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
  cancellation, poison-block quarantine, and a fixed replay-receipt hash.

This does not prove action-bearing account or order state, RocksDB durability,
reconciliation, live-source compatibility, a production replay process or CLI,
or Stage 2 readiness.
