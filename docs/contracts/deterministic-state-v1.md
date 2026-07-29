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

## Checkpoint identity

The in-memory checkpoint binds:

- chain ID;
- applied block height;
- canonical block hash;
- deterministic state hash; and
- reducer-set version.

It is not yet a durable checkpoint. The filesystem manifest, state-image
encoding/decoder, archive manifest binding, schema fingerprint, atomic
publication, restore validation, and RocksDB adapter remain later milestones.

## Current evidence and limitations

Focused tests prove:

- deterministic state bytes and a fixed hash vector;
- contiguous empty committed blocks;
- identical output across independent ledger instances;
- independence from reducer mutation emission order;
- whole-block rollback after a late event or invariant failure;
- exact duplicate idempotence and same-height divergence;
- chain, height, confirmation, reducer-version, and support gates; and
- mutation bounds and ambiguous key rejection.

This does not prove action-bearing account or order state, checkpoint crash
safety, archive replay, RocksDB durability, reconciliation, live-source
compatibility, or Stage 2 readiness.
