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

`CanonicalTradeReducerV2` is an additive enriched-trade fact reducer under
version `hyperliquid-alpha-desk-canonical-trade@2.0.0`. It supports only exact
`TradeMatched@1.0.0` events whose canonical payload retains buyer then seller
participant anchors matching the envelope account order. It emits only:

- one `trade.v2` record retaining both accounts, signed starting positions,
  order identities, explicit optional TWAP/client-order identities, price,
  quantity, market, block, and payload hash;
- two `trade-participant.v2` records, with ordinal 0 as buyer and a positive
  quantity effect and ordinal 1 as seller and a negative quantity effect; and
- one passed `trade-reconciliation.v2` record under
  `trade-position-symmetry@2.0.0`, binding equal absolute quantities and
  opposite signed effects.

All four target keys are computed and any existing values are strictly decoded
and key-bound before collision is reported. Corrupt, noncanonical, or
wrong-key prior facts fail closed with `trade_state.prior_fact_invalid`; valid
prior facts fail as an immutable trade-identity collision. No V2 mutation is
returned unless every prior-fact check and every record encoding succeeds.

`CanonicalTradeReducerSetV2` freezes the composite version
`hyperliquid-alpha-desk-canonical-trade-set@2.0.0`. Participant-free legacy
trades run through V1 only. Enriched trades run through V1 and V2 against the
same pre-event state, preserving the byte-exact V1 surface while adding V2
facts. It never accepts a direct V1 or direct V2 component checkpoint under the
composite version; recovery must rebuild from immutable archive events.
The synthetic mixed replay binds archive producer identity
`state-replay-trade-e2e-v2` and parser version
`state-replay-trade-fixture-v2`; the report records both rather than retaining
V1 provenance names under V2 semantics.

These V2 records retain source-observed anchors and per-trade signed effects.
They do not themselves maintain current account-market position quantity,
prove continuity between adjacent `start_position` values, infer maker/taker,
or qualify a deployed source. Optional identities serialize explicitly as JSON
values or `null`; client-order identities accept only the canonical lowercase
`0x` plus 32 lowercase hexadecimal digits.
Hard-coded literal byte vectors freeze all four V1 and all four V2 values,
including field order, hashes, lowercase roles, and explicit optional nulls;
the reducer output and key-bound decoders must match those vectors exactly.

`CanonicalPositionReducerV1` reconstructs exact source-anchored
account-market position quantity under reducer-set version
`hyperliquid-alpha-desk-canonical-position@1.0.0`. It permanently owns only
participant-bearing `TradeMatched@1.0.0`; participant-free legacy trades and
all order-fill, liquidation, settlement, and funding events remain outside its
support boundary.

An enriched trade is accepted only when its payload and envelope identities
match buyer then seller exactly and an existing key-bound
`market-current.v1` record provides exact tick, lot, price-scale, and
quantity-scale metadata. The reducer expands price, fill, both source start
positions, and both result positions upward to those active scales without
rounding. It then requires tick-aligned price and lot-aligned fill, starts, and
results. Exact quote-notional multiplication occurs after normalization as a
validation boundary; Task 4 does not persist that value. Buyer adds the fill
and seller subtracts it with checked signed fixed-point arithmetic.

The first observed trade accepts each source start position as authoritative
and records `first_observation`. A later known current must match the next
source start exactly after upward normalization and records `continued`. A
valid seeded unresolved current accepts the source start as a new
authoritative anchor and records `reanchored_from_unresolved`; its immutable
unresolved-cause facts remain untouched. A never-anchored unresolved current
sets its first anchor to that re-anchoring trade, while a previously anchored
current preserves its original first anchor.

Each accepted trade emits two immutable `position-effect-fact.v1` records,
keyed by framed trade identity and lowercase buyer/seller role, plus two
replaceable `position-quantity-current.v1` records keyed by framed raw account
bytes and market identity. `position-unresolved-cause-fact.v1` freezes the
lowercase `backstop_liquidation` cause and frames account, market, event, and
liquidation identities. Task 4 defines and validates this cause record but
does not create it or consume backstop events; the distinct later liquidation
reducer owns those mutations.

All position keys use checked u64 big-endian length frames. Key builders cap
preallocation at 64 KiB, record codecs cap canonical field-ordered JSON at
16 KiB, `decode_at` binds every stored identity to its key, and production
ledger mutation limits impose a stricter 4 KiB encoded-key ceiling. Existing
effect values are decoded and key-bound before a valid immutable collision is
reported. Any late invalid leg or event rejects the whole candidate block.
These records prove deterministic arithmetic over synthetic source-declared
anchors; they do not prove deployed source authority, protocol-reported
realized PnL, margin, liquidation, or live execution state.

`CanonicalPositionEpisodeReducerV1` adds an analytical episode projection
under reducer-set version
`hyperliquid-alpha-desk-canonical-position-episode@1.0.0`. It owns only exact
participant-bearing `TradeMatched@1.0.0`, `FundingPaid@1.0.0`, and
`FundingReceived@1.0.0`. Fees remain outside this projection because V1 fee
events carry no execution identity; time proximity is never used to attach
them to a position.

The quantity and episode reducers consume the same crate-private validated
trade result against the same pre-event state. That shared boundary performs
exact market lookup, upward-only scale normalization, tick/lot checks, signed
buyer/seller arithmetic, and full exact quote-notional multiplication.
Neither reducer consumes the other's same-event mutations. The paired current
state must satisfy exactly one of these conditions:

- both current records are absent;
- known zero quantity is paired with `no_open_episode`;
- known nonzero quantity is paired with a key-bound open episode; or
- unknown quantity is paired with `interrupted`.

Corrupt records, orphan currents, a missing or terminal resolved target, and
all cross-family mismatches fail closed. A known quantity must equal the next
source start after exact upward normalization. An absent or interrupted pair
may re-anchor from that source start.

An episode opened from observed flat state is
`complete_from_flat`. A first observation or unresolved re-anchor at nonzero
position is `partial_from_first_observation`, preserving the explicit opening
position without manufacturing earlier basis. Buyer activity always
increments observed buy quantity/notional and seller activity always
increments observed sell quantity/notional, whether the fill adds, reduces,
closes, or reverses the position. Quantities accumulate only after exact
upward scale alignment. Notionals retain exact integer coefficient and scale
pairs; canonical state never divides to derive VWAP.

A flat result closes the active episode as `trade_flat`. A reversal uses
checked signed magnitude, assigns only the closing quantity and notional to
ordinal 0, closes it as `trade_reversal`, and opens the residual from flat as
ordinal 1. The two quantities and two exact notionals must conserve the full
fill exactly. Episode identities are BLAKE3-derived from length-framed account,
market, opening event, and opening ordinal under the frozen V1 context.
Immutable per-event effects are keyed by event, account, market, and leg
ordinal. Existing effect and newly derived episode identities are decoded and
key-bound before a valid collision is reported; identical prior bytes are
still collisions.

Funding is attributed only when the paired current state is known nonzero and
resolves to an open episode. It exactly accumulates the named paid or received
side after upward-aligning the existing paid total, existing received total,
and incoming amount to their common maximum scale. Both persisted totals and
both effect deltas use that scale. It emits a zero-trade-delta episode effect
and refreshes provenance.
Funding observed before any position, while flat, or while attribution is
interrupted produces no episode bytes. The independent account reducer still
records its funding flow, including suppressed episode-attribution cases.

`observed_signed_trade_notional_delta` is available only for
`complete_from_flat` episodes closed by `trade_flat` or `trade_reversal`. It is
the checked exact difference `sell_notional - buy_notional`; it is not
source-reported realized PnL and excludes fees and funding. Partial, open, and
interrupted episodes return no metric. Repeated clean replay and checkpoint
resume are required to reproduce byte-identical episode state.

`CanonicalLiquidationReducerV1` owns exact schema `1.0.0`
`LiquidationStarted`, `LiquidationFill`, `BackstopLiquidation`, and
`PositionSettled` events under reducer-set version
`hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0`. These direct
canonical observations do not require retained market metadata. Each event is
reduced atomically from its pre-event state; a later event in the same block
sees the complete candidate state produced by earlier events.

A start writes an immutable source fact and a globally unique liquidation
current. Fills and backstops require that current, the exact liquidated
account, and strictly advancing block, transaction, and canonical-event
position with a distinct event identity. A retained fill flow must be
coherent with the process observation: equal positions require the same event
identity, otherwise the flow position and identity are strictly earlier. The
incoming fill must also have a distinct identity and position strictly after
the flow. Incoherent flow provenance fails before account-pair validation or
quantity arithmetic. A fill preserves process status and first-backstop provenance,
advances its last observation, and accumulates source quantity only in its
exact liquidation/account/market flow. Flow inputs are rescaled upward to
their maximum scale and added exactly. Backstop quantities never enter fill
flow. The first backstop changes status to `backstop_observed`; later
backstops preserve the first provenance.

Known signed positions are reduced toward zero without computing an absolute
signed magnitude: a long subtracts the incoming unsigned quantity and a short
adds it. Inputs are rescaled upward to their maximum scale with checked exact
arithmetic. A liquidation fill that crosses zero, or targets known flat state,
rejects atomically as `liquidation_state.fill_overrun`. Absent or already
unknown state admits the source fact, process observation, and flow but leaves
quantity unknown. Settlement uses the same exact arithmetic when direction is
known; absent, unknown, known-flat, or overrun settlement is instead admitted
as unknown attribution. Settlement remains independent of liquidation process
state, writes no liquidation flow or account quote flow, and retains signed
source realized PnL only in its immutable fact.

Every non-trade quantity transition interrupts a resolved old episode. Exact
flat state leaves `no_open_episode`; an exact nonzero remainder opens a new
`partial_from_first_observation` episode at ordinal `1`; ambiguous state leaves
`interrupted`. The old interruption and zero-valued effect use ordinal `0`.
These effects never close an episode and never manufacture basis, notional,
funding, entry price, transfer price, or realized PnL. Quantity zeros retain
the transition scale, funding zeros use scale zero, and notional zeros encode
canonically as `0`.

A backstop processes the literal account order `[liquidated, backstop]`.
For each account it writes an immutable unresolved-cause fact, preserves any
first position anchor, sets quantity unknown, and interrupts a resolved
episode. An unseen account receives an unknown quantity/current pair without a
fabricated anchor. Preparation failure for either account rejects the complete
fact, process, and both account bundles.

Mutation order is fact, fill flow when present, liquidation current when
present, then each account bundle as unresolved cause when present, ordinal
`0`/`1` effects, old/new episode records, quantity current, and episode
current. Existing immutable identities are decoded and key-bound before valid
collisions are reported, proposed quantity/episode pairs are validated before
return, and duplicate mutation keys fail closed. Fresh replay, whole-block
repeat, and exact-version checkpoint resume must reproduce identical canonical
state bytes and hashes; reducer-set version substitution is refused.

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

`CanonicalAccountReducerV1` reconstructs the source-observed account flows,
direct relations, and asserted mode transitions under reducer-set version
`hyperliquid-alpha-desk-canonical-account@1.0.0`. It owns only exact schema
`1.0.0` events for deposits, withdrawals, spot/perpetual/subaccount transfers,
vault deposits and withdrawals, fees, builder fees, funding paid and received,
referral rewards, account-mode changes, margin-mode changes, and leverage
changes.

Every accepted event creates one immutable `account-fact.v1` record. Facts
retain the exact ordered envelope account and market identities, including
literal duplicates, together with the event kind, payload asset or vault
identity where present, block height, payload hash, and reducer version.
Missing, extra, reordered, or deduplicated envelope identities fail closed. A
fact key that already exists is an immutable event-identity collision; it is
never overwritten.

Current records use these exact namespaces:

- `account-quantity-flow-current.v1` for asset and vault-share `Quantity`
  credits and debits;
- `account-quote-flow-current.v1` for default perpetual quote, market funding,
  and vault-principal `QuoteAmount` credits and debits;
- `vault-principal-flow-current.v1` and `vault-share-flow-current.v1` for
  observed vault deposit/withdrawal principal and issued/redeemed shares;
- `account-subaccount-master.v1` for one observed direct master per
  subaccount;
- `account-vault-relation.v1` for an observed account and vault interaction;
  and
- `account-mode-current.v1`, `account-margin-mode-current.v1`, and
  `account-leverage-current.v1` for predecessor-bound asserted settings.

Quantity and quote records are separate Rust types and namespaces. Existing
totals and an incoming amount are normalized only upward to their greatest
scale before checked addition. This is exact and never rounds or downscales.
Overflow, impossible scale expansion, corrupt current bytes, key mismatch, or
two mutations for the same event key rejects the complete candidate block.
Deposits and withdrawals affect only external-asset flow. Transfer and builder
fee legs are equal debit/credit observations in their source-proven scopes.
Fee direction follows the frozen fee type: maker rebates credit; every other
supported fee type debits. Funding direction follows the event kind, not the
sign of the separately retained funding rate. Referral rewards credit only the
explicit referrer; no debit source is invented.

Vault events atomically emit account principal, account shares, observed vault
principal, observed vault shares, and the observed relation. Account and vault
amounts reconcile separately within the principal and share units; principal
is never compared with shares. A subaccount transfer establishes or refreshes
exactly one direct relation only when the asserted master is exactly one
transfer endpoint. Three-distinct-account scope, a conflicting later master,
or an inferred transitive hierarchy is rejected. First mode and leverage
events retain the asserted predecessor; later events must name the current
value exactly. Legal cycles back to the initially asserted predecessor remain
valid.

Every account event carrying an asset identity requires a current
`asset-context-current.v1` record decoded at its exact key. Funding,
margin-mode, and leverage events similarly require a key-bound
`market-current.v1` record with exact metadata resolution. Missing, corrupt,
key-mismatched, or unresolved prerequisites fail closed. Perpetual transfers
and vault events do not carry an authoritative asset or market identity, so
the reducer does not synthesize either prerequisite.

These records are observed-flow state only. They do not establish an opening
balance, current venue balance, current vault holding or share supply,
clearinghouse route, complete account hierarchy, authoritative
portfolio-margin state, position, PnL, liquidation price, or deployed-source
qualification. Source-anchored position quantity is a separate reducer
contract described above; the production market/order/trade/account/position
composite remains later work.

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
- frozen V1 canonical trade facts and enriched V2 buyer/seller anchors, exact
  signed effects, and stored position-symmetry records; malformed late-event
  whole-block rollback; valid, corrupt, and key-mismatched prior-fact
  rejection; bounded canonical codec/key binding; and mixed legacy/enriched
  archive replay/checkpoint equivalence; and
- exact synthetic source-anchored position quantity with buyer/seller
  symmetry, first/continued/unresolved re-anchor transitions, strict
  market-scale/tick/lot prerequisites, checked arithmetic, immutable effects,
  preserved unresolved causes, key-bound bounded codecs, deterministic mixed
  legacy/enriched block application, and late-event whole-block rollback; and
- a bounded operator-visible synthetic trade runner proving repeated mixed
  rebuild, per-version decoded record cardinality, private composite checkpoint
  resume, store-level direct V1/V2 component-checkpoint publication and exact
  incompatibility refusal, malformed-trade reducer failure,
  unsupported-schema quarantine, and private evidence publication; and
- exact order acceptance, resting, modification, partial fill, terminal fill,
  cancellation, and rejection state; immutable fact and hash-linked transition
  records; strict identity/key/codec binding; checked overfill and remainder
  rejection; terminal-state non-resurrection; and whole-block rollback after a
  late invalid order transition; and
- a bounded operator-visible synthetic order runner proving repeated rebuild,
  decoded lifecycle cardinality, private checkpoint resume, late-overfill
  reducer failure, unsupported-schema quarantine, owner-only evidence
  permissions, and explicit false Stage 1/2, deployed/live source, position,
  margin, and execution qualification; and
- a bounded operator-visible synthetic market runner proving prerequisite-
  ordered creation, valuation, cap/table, halt/resume, and outcome transitions;
  at least two independent full-range replays that include the metadata
  transition and produce identical unresolved final-state and receipt hashes;
  strict decoding of both exact and unresolved metadata intervals after every
  full replay; a private checkpoint resume whose suffix crosses the same
  transition and reaches the same unresolved final hash; exact-to-unresolved
  interval closure and cleared applicability; suppressed value updates with
  `market_state.metadata_unresolved`; late invalid-transition whole-block
  rollback; schema `1.1.0` quarantine; and recursively owner-only evidence
  permissions.

This proves stored V1 canonical trade-fact reconciliation, synthetic
source-declared V2 buyer/seller anchors and effects, exact synthetic
source-anchored position-quantity continuity, and exact synthetic
order-lifecycle and market-registry contracts. It does not prove deployed
action-bearing source compatibility, maker/taker roles, authoritative market
metadata, external oracle or snapshot reconciliation, authoritative live
position, analytical episodes, margin, book, signal, or execution state,
RocksDB durability, a production replay service, or Stage 2 readiness. The
retained local order report at
`target/evidence/state-replay-order/20260729T185537Z-82818/report.json` covers
20 generated blocks, four independent rebuilds, a checkpoint after block 8,
80 facts and transitions, 20 current orders, 10 filled orders, 10 cancelled
orders, and 10 fact-only rejections. The retained local market report at
`target/evidence/state-replay-market/20260729T211159Z-40107/report.json` covers
20 generated blocks, four independent rebuilds, a checkpoint after block 8,
119 facts, one DEX, two asset contexts, one active current market with
unresolved metadata, two metadata versions spanning one closed exact interval
and one open unresolved interval, one resolved outcome, metadata-unresolved
value suppression, and atomic invalid and unsupported rejection. Runnable
replay evidence remains generated canonical-event evidence with source
qualification explicitly unassessed.
