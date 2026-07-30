# Canonical Trade Positions and Analytical Episodes Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` or
> `superpowers:executing-plans`. Every task uses red-green-refactor, an exact
> ownership range, an independent review, and a scoped commit.

**Goal:** Reconstruct exact per-account position quantity and source-observed
trade effects from the documented node trade schema, then build analytical
position episodes without assuming an opening flat position, inventing a
protocol cost basis, rounding a non-terminating VWAP into canonical state, or
attributing fees to a market without an execution identity.

**Architecture:** Enrich the canonical `TradeMatched` contract with the
documented buyer/seller side information already present in node trade rows.
Each trade remains one cross-account canonical object and is the sole owner of
position effects; order-fill events remain order-lifecycle facts. A position
reducer uses each participant's source-provided `start_pos` as an exact
pre-event anchor and applies the signed fill once. Protocol position quantity,
source-observed closed-PnL/fee evidence, and analytical episodes remain
separate record families. Entry/exit VWAP is stored as exact notional and
quantity components, never as a rounded canonical decimal.

**Tech Stack:** Rust 1.97.1, Prost canonical V1, bounded fixed-point/wide
decimal types, BLAKE3 state hashing, strict key-bound JSON records, immutable
archive/checkpoint replay.

## Why the previous Task 5 is on HOLD

The earlier position task was reviewed before implementation and found unsafe:

- the current `TradeMatched` drops the documented buyer/seller starting
  positions and order identities;
- treating the first retained fill as an opening-from-flat event would invent
  an opening position;
- consuming both `TradeMatched` and order-fill events would double count;
- weighted average entry can be a non-terminating rational, so an exact
  `Price` field cannot represent every valid sequence without rounding;
- `FeeCharged` has no market/order/trade identity and uses asset `Quantity`,
  so it cannot populate a quote-denominated position fee;
- liquidation/backstop payloads do not prove side, transfer price, recipient
  cost basis, or a settlement-to-liquidation link.

The official
[L1 data schema](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/nodes/l1-data-schemas)
documents `side_info[0]` as the buyer and `side_info[1]` as the seller, each
with `user`, `start_pos`, `oid`, optional `twap_id`, and optional `cloid`. The
node can also write fills in API-fill format, but the published fill example
and the current repository fixture do not carry an account identity. Node-fill
mapping therefore remains evidence-only until a retained real recording
proves the account/join key.

The official
[entry-price and PnL documentation](https://hyperliquid.gitbook.io/hyperliquid-docs/trading/entry-price-and-pnl)
also states that entry price, unrealized PnL, and closed PnL are frontend
convenience components while fundamental accounting is based on margin and
trades. This plan keeps exact protocol-observed position quantity separate
from analytical entry/PnL views.

## Global constraints

- V1 remains read-only: no signer, private key, order placement, custody, or
  execution route.
- Source qualification remains `synthetic_unassessed` until immutable real
  node recordings are retained and mapped.
- A node trade produces both account effects from one canonical trade object.
  `OrderPartiallyFilled` and `OrderFilled` never mutate position state.
- Buyer position result is `buyer_start_position + quantity`; seller result is
  `seller_start_position - quantity`, after exact upward scale normalization.
- An exact existing current quantity must equal the next source `start_pos`;
  mismatch quarantines the block. When quantity is unresolved after a
  backstop, the next participant-bearing trade re-anchors it from source
  `start_pos` and opens a partial analytical episode unless that start is
  flat.
- No first-observation-from-flat assumption. A nonzero first `start_pos`
  establishes exact current quantity but only a partial analytical episode.
- No canonical division or implicit rounding. Store exact notional numerator
  and quantity denominator components.
- `FeeCharged` remains account/asset flow only. Position fee attribution
  requires a typed account fill with account, market, trade/order, fee token,
  and amount.
- `FundingPaid`/`FundingReceived` may attach to an exact open account-market
  episode, but remain account-market flow when no exact episode is open.
- Backstop events record both accounts and immutable unresolved-cause facts;
  they never manufacture quantity, price, side, entry basis, or PnL. Multiple
  unresolved causes are preserved independently.
- Every block is atomic and every immutable effect key rejects collision.
- The existing node-trade mapper emits `ProvisionalSource`, which the ledger
  must continue rejecting. Tasks 1-8 prove synthetic reducer semantics only;
  production remains blocked on an exact auxiliary-trade-to-committed-block
  join and source qualification gate.

---

### Task 1: Add explicit signed position quantity and bounded exact notional

**Files:**
- Modify: `crates/domain-types/src/decimal.rs`
- Modify: `crates/domain-types/src/lib.rs`
- Modify: `crates/domain-types/tests/decimal.rs`
- Create: `crates/domain-types/tests/exact_notional.rs`

**Produces:**

```rust
pub struct PositionQuantity(Decimal);

pub struct ExactQuoteNotional {
    coefficient: BigInt,
    scale: u8,
}
```

`PositionQuantity` is explicitly signed and bounded by the existing decimal
scale limit. `ExactQuoteNotional` is a signed, bounded wider representation
for the exact product of `Price * Quantity`, with:

```text
MAX_NOTIONAL_SCALE = 76
MAX_NOTIONAL_COEFFICIENT_BITS = 512
MAX_NOTIONAL_DECIMAL_DIGITS = 155
MAX_NOTIONAL_WIRE_BYTES = 256
```

Coefficient bit length is the magnitude bit length and must be at most 512.
Custom parsing checks the 256-byte and 155-decimal-digit bounds before
constructing a `BigInt`, then enforces the bit bound. Custom Serde is
mandatory; derived deserialization may not bypass invariants. Canonical form
has no plus sign or exponent, no leading integer zeros, no trailing fractional
zeros, and one zero spelling (`"0"` at scale zero). Every constructor and
multiplication, rescale, addition, and subtraction rechecks the scale and
coefficient bounds. The type supports exact upward rescaling only and exposes
no canonical division.

- [x] Write red tests for positive/negative/zero position quantities,
  scale-38 boundaries, exact price-times-quantity products, sign behavior,
  scale-sum bounds, 511/512/513-bit coefficients, pre-parse byte/digit limits,
  operation-result overflow, exact upward normalization, canonical zero/sign/
  scale and String/Serde forms, malformed/noncanonical input, and deterministic
  ordering.
- [x] Run focused red tests and retain the missing-type signal.
- [x] Implement the smallest types with no `f32`/`f64`, unbounded allocation,
  rounding, or lossy conversion.
- [x] Run domain tests, strict all-target/all-feature Clippy, formatting, and
  diff checks.
- [x] Commit with `feat(domain): add exact position accounting values`.

---

### Task 2: Enrich the canonical trade participant contract

**Files:**
- Modify: `crates/domain-types/src/ids.rs`
- Modify: `crates/domain-types/src/lib.rs`
- Modify: `crates/domain-types/tests/ids.rs`
- Modify: `schemas/proto/canonical/v1/events.proto`
- Modify: `crates/api-contracts/src/lib.rs`
- Modify: `crates/api-contracts/tests/payload_codec.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Modify: `crates/canonical-events/src/node_mapping.rs`
- Modify: `crates/canonical-events/tests/node_mapping.rs`
- Modify: `crates/canonical-events/tests/payload.rs`
- Modify: `crates/canonical-ledger/tests/trade_state.rs`
- Modify: `crates/replay-engine/tests/serial_replay.rs`
- Modify: `tools/state-replay/src/trade.rs`
- Modify: `crates/telemetry/schema-fingerprint-v1.material`
- Modify: `fixtures/canonical/node-v1/expected.json`
- Modify: generated schema artifacts only through the repository's
  deterministic generator; the compatibility baseline descriptor remains
  unchanged because the extension is additive.

**Produces:**

```rust
pub enum TradeParticipantRoleV1 {
    Buyer,
    Seller,
}

pub struct TradeParticipantV1 {
    pub role: TradeParticipantRoleV1,
    pub account_id: Address,
    pub start_position: PositionQuantity,
    pub order_id: OrderId,
    pub twap_id: Option<TwapId>,
    pub client_order_id: Option<ClientOrderId>,
}

pub struct TradeMatched {
    // existing identity, price, quantity, and seed fields
    pub participants: Option<[TradeParticipantV1; 2]>,
}
```

The canonical order is buyer then seller and must match the exact envelope
account list. The optional container preserves decode compatibility for prior
synthetic V1 envelopes, but any source-qualified position effect requires
`Some([Buyer, Seller])`. Do not reinterpret the existing maker/taker order IDs
as buyer/seller IDs.

- [x] Write red tests for exact participant order, roles, account-envelope
  binding, signed start positions, positive fill quantity/price, distinct
  accounts, required order IDs, optional TWAP/client IDs, 16 KiB preflight,
  unknown-field preservation through the enclosing envelope, and deterministic
  re-encoding.
- [x] Extend `NodeTrade` mapping to parse all documented `side_info` fields.
  Reject wrong array length, invalid start positions, missing order IDs,
  malformed optionals, or account mismatch. Preserve source index order and
  bind index 0 to Buyer and index 1 to Seller; the source has no independent
  role tag with which to detect producer-side swapping. Never infer
  maker/taker.
- [x] Keep source evidence and original record bytes/hash unchanged.
- [x] Regenerate schema artifacts deterministically and pass generated-drift
  checks.
- [x] Run domain/API/event focused and full tests, strict Clippy, formatting,
  and diff checks.
- [x] Commit with `feat(events): retain canonical trade participants`.

---

### Task 3: Make trade facts retain exact participant anchors

**Files:**
- Modify: `crates/canonical-ledger/src/lib.rs`
- Modify: `crates/canonical-ledger/src/trade.rs`
- Modify: `crates/canonical-ledger/tests/trade_state.rs`
- Modify: `tools/state-replay/src/lib.rs`
- Modify: `tools/state-replay/src/main.rs`
- Modify: `tools/state-replay/src/trade.rs`
- Modify: `tools/state-replay/tests/trade_e2e.rs`
- Modify: `tools/state-replay/tests/cli.rs`
- Modify: `docs/runbooks/state-replay-evidence.md`
- Modify: `docs/contracts/deterministic-state-v1.md`

**Produces:**

- `TradeStateRecordV2` in namespace `trade.v2` that retains
  buyer/seller accounts, start positions, order IDs, optional TWAP/client IDs,
  price, quantity, market, and payload hash.
- Two `trade-participant.v2` facts keyed by trade and ordinal, with exact role
  and start-position data.
- A `trade-reconciliation.v2` record for the two-sided effect.
- `CanonicalTradeReducerV2::VERSION =
  "hyperliquid-alpha-desk-canonical-trade@2.0.0"`.
- Reconciliation evidence that buyer and seller receive equal absolute
  quantity with opposite signs.

Do not change `trade.v1`, its reducer, or its persisted record bytes.
`CanonicalTradeReducerV2` decodes enriched events into V2 namespaces.
Participant-free legacy events remain accepted by the V1 fact path only.
Enriched events run through both reducers: V1 preserves the existing
byte-compatible fact surface and V2 adds the participant-bearing state.
`CanonicalTradeReducerSetV2` owns that composition with exact version
`hyperliquid-alpha-desk-canonical-trade-set@2.0.0`; both component reducers
evaluate the same pre-event state before their disjoint mutations are
concatenated atomically. Mixed replay reports and checkpoints bind to the set
version and reject V1 or direct-V2 component checkpoints. Recovery rebuilds
from the immutable archive, never by accepting an old state hash under new
semantics. Preserve byte-exact V1 decode fixtures and retain a mixed
V1/enriched archive replay fixture.

- [x] Write red codec/reducer/migration tests first.
- [x] `CanonicalTradeReducerV2` supports only participant-bearing enriched
  trades. Preserve fact-only V1 behavior for prior synthetic envelopes.
- [x] Reject trade-ID collision and corrupt/key-mismatched prior facts.
- [x] Regenerate all affected replay report fixtures and bind report schemas
  to the V2 reducer-set version; refuse V1 and direct-V2 component checkpoints
  under composite restore.
- [x] Run ledger/replay tests and strict gates.
- [x] Commit with `feat(ledger): retain trade position anchors`.

---

### Task 4: Reconstruct exact source-anchored position quantity

**Files:**
- Create: `crates/canonical-ledger/src/position/mod.rs`
- Create: `crates/canonical-ledger/src/position/codec.rs`
- Create: `crates/canonical-ledger/src/position/quantity.rs`
- Modify: `crates/canonical-ledger/src/lib.rs`
- Create: `crates/canonical-ledger/tests/position_quantity.rs`
- Modify: `docs/contracts/deterministic-state-v1.md`

**Produces:**

```rust
pub struct PositionQuantityCurrentRecordV1 {
    account_id: Address,
    market_id: MarketId,
    known_quantity: Option<PositionQuantity>,
    first_anchor_event_id: Option<EventId>,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

pub enum PositionUnresolvedCauseV1 {
    BackstopLiquidation,
}

pub enum PositionAnchorTransitionV1 {
    FirstObservation,
    Continued,
    ReanchoredFromUnresolved,
}

pub struct PositionUnresolvedCauseFactRecordV1 {
    account_id: Address,
    market_id: MarketId,
    event_id: EventId,
    liquidation_id: LiquidationId,
    cause: PositionUnresolvedCauseV1,
}

pub struct PositionEffectFactRecordV1 {
    event_id: EventId,
    trade_id: TradeId,
    account_id: Address,
    market_id: MarketId,
    role: TradeParticipantRoleV1,
    anchor_transition: PositionAnchorTransitionV1,
    start_position: PositionQuantity,
    fill_quantity: Quantity,
    result_position: PositionQuantity,
    rule_version: String,
}
```

`PositionUnresolvedCauseV1` has the exact lowercase wire variant
`backstop_liquidation`; adding another cause is a schema-version change.
`PositionAnchorTransitionV1` has exact lowercase wire variants
`first_observation`, `continued`, and `reanchored_from_unresolved`; adding
another transition is a schema-version change. Record fields remain private
and invariant-bearing records expose validated decoders and read-only
accessors.

Freeze these identities:

```text
CanonicalPositionReducerV1::VERSION =
  hyperliquid-alpha-desk-canonical-position@1.0.0

position-quantity-current.v1
  hyperliquid-alpha-desk/position-quantity-current/v1
  key: frame(raw account) + frame(market_id)

position-effect-fact.v1
  hyperliquid-alpha-desk/position-effect-fact/v1
  key: frame(trade_id) + frame(role)

position-unresolved-cause-fact.v1
  hyperliquid-alpha-desk/position-unresolved-cause-fact/v1
  key: frame(raw account) + frame(market_id) + frame(event_id)
       + frame(liquidation_id)
```

Every `frame` is checked `[u64 big-endian length][bytes]`. The exact effect
`rule_version` is `CanonicalPositionReducerV1::VERSION`. Roles are lowercase
`buyer` and `seller`. Codecs accept at most 16 KiB of canonical JSON, key
builders precompute and `try_reserve_exact` at most 64 KiB, and reducer
mutations remain subject to the production ledger's 4 KiB encoded-key limit.
Existing immutable effect/unresolved values must pass `decode_at` before a
normal identity collision is reported.

Freeze reducer reason codes:

```text
position_state.unsupported_event
position_state.identity_mismatch
position_state.market_prerequisite_missing
position_state.market_prerequisite_invalid
position_state.market_metadata_unresolved
position_state.scale_normalization
position_state.price_tick_misaligned
position_state.quantity_lot_misaligned
position_state.notional_arithmetic
position_state.position_arithmetic
position_state.current_record_invalid
position_state.start_position_mismatch
position_state.effect_collision
position_state.unresolved_cause_collision
position_state.duplicate_mutation_key
position_state.codec.invalid_key
position_state.codec.decode
position_state.codec.noncanonical
position_state.codec.invalid_record
position_state.codec.key_mismatch
position_state.codec.limit_exceeded
```

Record invariants are:

- `known_quantity = Some` requires `first_anchor_event_id = Some`;
- `known_quantity = None` with no first anchor means never anchored;
- `known_quantity = None` with a first anchor means previously anchored and
  later unresolved; and
- `known_quantity = Some` with no first anchor is invalid.

`CanonicalPositionReducerV1::supports` returns true only for exact-schema
participant-bearing `TradeMatched`; the composite skips participant-free
legacy trades while the V1 trade reducer retains their fact-only behavior.
Direct invocation on a participant-free trade returns unsupported-event.
This reducer remains trade-only permanently. Task 6B owns backstop creation
and position invalidation through a distinct non-trade reducer/version.
Task 4 defines and tests the unresolved-cause/current codecs and may seed
valid unresolved state solely to prove later trade re-anchoring; it does not
consume `BackstopLiquidation`.

One enriched `TradeMatched` event emits two immutable effects and two current
updates.
On first observation, accept the source start position as the anchor. On later
events with known quantity, require it to equal source start position exactly
after upward scale normalization. Buyer adds and seller subtracts. A flat
result is exact zero at the normalized scale. When prior quantity is
unresolved, the source `start_pos` is an authoritative re-anchor; preserve all
unresolved-cause facts and record that this trade replaced unresolved state.
An unseen backstop current has `first_anchor_event_id = None`; the first later
participant-bearing trade sets it to `Some(event_id)`. Task 4 never creates or
mutates analytical episodes; Task 5B consumes the same trade/pre-event state
and owns the corresponding partial-or-complete episode transition.

- [ ] Write red tests for first nonzero anchor, buyer/seller symmetry, long and
  short add/reduce/flat/reversal, mixed scales, overflow, missing participants,
  reordered identities, unresolved metadata, duplicate effect, start-position
  mismatch, seeded unresolved-state re-anchor, all four
  known-quantity/first-anchor combinations, corrupt current state, mixed
  legacy/enriched test-dispatcher application, and late block rollback.
- [ ] Require exact current market metadata. Normalize price to the active
  price scale and fill/start/result quantities to the active quantity scale
  only by exact upward rescaling; then require price tick alignment and fill,
  both starts, and both results to be lot-aligned. Perform exact notional
  multiplication only after this normalization. The notional is
  validation-only in Task 4; `trade.v2` owns the source price and Task 5B owns
  analytical notional persistence.
- [ ] Ensure order-fill events are unsupported by this reducer and cannot
  double count.
- [ ] Add strict key-bound 16 KiB codecs and 64 KiB preallocation bounds;
  test the ledger's stricter 4 KiB mutation-key ceiling separately.
- [ ] Use a test-only dispatcher that evaluates market,
  `CanonicalTradeReducerSetV2`, and `CanonicalPositionReducerV1` against the
  same pre-event state, skips position reduction for legacy trades, and
  rejects cross-child key collisions. Production mixed archive/checkpoint
  replay remains Task 7/8 under `CanonicalStateReducerV1::VERSION =
  "hyperliquid-alpha-desk-canonical-state@1.0.0"`.
- [ ] Run ledger/replay tests and strict gates.
- [ ] Commit with `feat(state): reconstruct anchored position quantity`.

---

### Task 5A: Add analytical episode records and codecs

**Files:**
- Create: `crates/canonical-ledger/src/position/episodes.rs`
- Create: `crates/canonical-ledger/tests/position_episode_records.rs`
- Modify: `crates/canonical-ledger/src/position/mod.rs`

**Produces:**

```rust
pub enum EpisodeCompletenessV1 {
    CompleteFromFlat,
    PartialFromFirstObservation,
}

pub enum EpisodeCloseCauseV1 {
    TradeFlat,
    TradeReversal,
    LiquidationFill,
    Settlement,
    BackstopInterrupted,
}

pub enum EpisodeStatusV1 {
    Open,
    Closed,
    Interrupted,
}

pub enum EpisodeAttributionResolutionV1 {
    NoOpenEpisode,
    Resolved,
    Interrupted,
}

pub struct PositionEpisodeRecordV1 {
    pub episode_id: PositionEpisodeId,
    pub account_id: Address,
    pub market_id: MarketId,
    pub opening_anchor_event_id: EventId,
    pub opening_leg_ordinal: u8,
    pub close_event_id: Option<EventId>,
    pub close_cause: Option<EpisodeCloseCauseV1>,
    pub completeness: EpisodeCompletenessV1,
    pub buy_quantity: Quantity,
    pub buy_notional: ExactQuoteNotional,
    pub sell_quantity: Quantity,
    pub sell_notional: ExactQuoteNotional,
    pub funding_paid: QuoteAmount,
    pub funding_received: QuoteAmount,
    pub status: EpisodeStatusV1,
}

pub struct PositionEpisodeCurrentRecordV1 {
    pub account_id: Address,
    pub market_id: MarketId,
    pub episode_id: Option<PositionEpisodeId>,
    pub attribution_resolution: EpisodeAttributionResolutionV1,
    pub last_event_id: EventId,
}
```

Derive `PositionEpisodeId` deterministically from account, market, opening
anchor event, and leg ordinal. Keys are:

```text
position-episode.v1                  frame(episode_id)
position-episode-current.v1          frame(account) + frame(market)
position-episode-effect-fact.v1      frame(event_id) + frame(account) + frame(market) + frame(leg_ordinal)
```

Leg ordinal is bounded to `0..=1`: `0` is the first or only episode effect for
an account-market event, and `1` is the second residual/reseed effect when the
same event closes or interrupts one episode and opens another. Trade reversal,
partial liquidation, and partial settlement therefore use effect ordinal `0`
for the old leg and ordinal `1` for the new residual/reseeded leg. The exact
lowercase wire variants are
`complete_from_flat` / `partial_from_first_observation`,
`trade_flat` / `trade_reversal` / `liquidation_fill` / `settlement` /
`backstop_interrupted`, `open` / `closed` / `interrupted`, and
`no_open_episode` / `resolved` / `interrupted`.

An open episode has neither close field. Closed and interrupted episodes have
both close fields. `Resolved` requires `Some(episode_id)` referencing an open
episode; `NoOpenEpisode` and `Interrupted` require `episode_id = None`. A
current pointer never references a closed or interrupted episode. Strict
key-bound codecs use the established 16 KiB value and 64 KiB preallocation
rules.

- [ ] Write red codec/key tests for deterministic IDs, both leg ordinals,
  current pointers, completeness, close event/cause pairing, closed/current
  inconsistency, exact-notional bounds, collision, key mismatch, canonical
  JSON, same-event two-effect key separation, 16 KiB values, and 64 KiB keys.
- [ ] Implement records/codecs only; no transition logic.
- [ ] Run focused/full ledger tests and strict gates.
- [ ] Commit with `feat(state): add position episode records`.

---

### Task 5B: Reduce analytical episodes without canonical division

**Files:**
- Modify: `crates/canonical-ledger/src/position/episodes.rs`
- Create: `crates/canonical-ledger/tests/position_episodes.rs`
- Modify: `crates/canonical-ledger/src/position/mod.rs`
- Modify: `docs/contracts/deterministic-state-v1.md`

An exact episode begins only from a source-proven flat start position. A first
nonzero anchor creates a partial observed episode. Flat closes the episode.
For reversal:

```text
closed_quantity   = min(abs(start_position), fill_quantity)
residual_quantity = fill_quantity - closed_quantity
```

The old/partial episode receives only the closing quantity and its notional at
the trade price. If residual is positive, leg ordinal `1` opens a new,
`CompleteFromFlat` opposite episode at the same price. This rule also applies
when the first retained trade reverses a nonzero source start position, so the
same event creates a partial closed leg `0` and a complete residual leg `1`
without key collision. Entry/exit VWAP remains the exact
`notional / quantity` pair; no canonical decimal division occurs.

For a `CompleteFromFlat` episode closed only by trades,
`observed_signed_trade_notional_delta = sell_notional - buy_notional`.
This is an analytical trade-only metric, not protocol/source `closedPnl`, and
excludes fees and funding. It is unavailable when liquidation, settlement,
backstop, or another non-trade cause changes quantity.

- [ ] Test flat-open-close, partial observation, add/reduce, full close,
  first-observation reversal, same-event two-episode identity, reversal split,
  non-terminating VWAP, wide overflow, duplicate effect, current-pointer
  consistency, and closed-episode immutability.
- [ ] Attribute funding only while an exact/partial open episode pointer is
  resolved. Suppress attribution when the pointer is interrupted/unresolved;
  retain the Task 4 account-market flow either way.
- [ ] Keep `FeeCharged` out of episodes; document the missing execution
  identity rather than assigning by time proximity.
- [ ] Run ledger/replay tests and strict gates.
- [ ] Commit with `feat(state): add exact analytical position episodes`.

---

### Task 6A: Add conservative liquidation and settlement fact records

**Files:**
- Create: `crates/canonical-ledger/src/position/liquidations.rs`
- Create: `crates/canonical-ledger/tests/liquidation_records.rs`
- Modify: `crates/canonical-ledger/src/position/mod.rs`

Implement strict key-bound codecs for:

```text
liquidation-current.v1
  key: frame(liquidation_id)

liquidation-start-fact.v1
  key: frame(liquidation_id) + frame(event_id)

liquidation-fill-fact.v1
  key: frame(liquidation_id) + frame(event_id)

liquidation-market-flow-current.v1
  key: frame(liquidation_id) + frame(account) + frame(market)

backstop-liquidation-fact.v1
  key: frame(liquidation_id) + frame(event_id)

position-settlement-fact.v1
  key: frame(event_id) + frame(account) + frame(market)
```

`LiquidationCurrentRecordV1` binds the started account, margin observation,
first/last events, and an observed process status with exact lowercase wire
variants `started` and `backstop_observed`. No V1 payload proves completion,
so there is no `completed`, `terminal`, or final-fill status. Market quantity
aggregates are always
separate per liquidation, account, and market; never sum across markets.
Backstop facts retain both accounts and the explicitly missing price/basis.
Settlement facts have no liquidation ID and therefore no process status.

```rust
pub enum LiquidationObservedStatusV1 {
    Started,
    BackstopObserved,
}
```

The exact wire variants are `started` and `backstop_observed`. Neither means
that the liquidation is complete or terminal.

- [ ] Write red codec/key tests for every record, process status, account
  binding, per-market separation, duplicates, canonical JSON, key mismatch,
  16 KiB values, and 64 KiB keys.
- [ ] Implement records only, run full ledger/replay/strict gates, and commit
  with `feat(state): add liquidation fact records`.

---

### Task 6B: Reduce liquidation, backstop, and settlement effects

**Files:**
- Modify: `crates/canonical-ledger/src/position/liquidations.rs`
- Create: `crates/canonical-ledger/tests/liquidation_state.rs`
- Modify: `crates/canonical-ledger/src/position/mod.rs`
- Modify: `crates/canonical-ledger/src/position/episodes.rs`
- Modify: `docs/contracts/deterministic-state-v1.md`

`LiquidationFill` records its immutable explicit-price quantity and may reduce
known position quantity only when the exact current sign makes direction
unambiguous. Every applied non-trade quantity mutation interrupts the active
episode, even when a same-side nonzero quantity remains. Flat leaves
`NoOpenEpisode`; a known nonzero result opens a new
`PartialFromFirstObservation` episode anchored at the liquidation-fill event;
an ambiguous result leaves attribution `Interrupted`. It never manufactures
entry basis.

`BackstopLiquidation` records both accounts, writes an independent unresolved
cause for each account/market even when no prior current record exists, sets
`known_quantity = None`, and marks the episode pointer interrupted so later
funding cannot attach to stale state. The next enriched trade may re-anchor
from its source `start_pos`; unresolved facts remain immutable history.

`PositionSettled` remains independent from liquidation process state. Freeze
this V1 rule: when current quantity is known and its sign makes reduction
unambiguous, reduce by `settled_quantity` toward zero and apply the explicit
source `realized_pnl` only to the immutable settlement fact; otherwise keep
the event fact-only and mark quantity unresolved. It may close or interrupt an
episode, but never closes a liquidation process.

Freeze settlement `realized_pnl` as settlement-fact-only in V1: it does not
create or update an account quote-flow family. Every settlement that mutates
known quantity interrupts the active episode; a known nonzero result opens a
new `PartialFromFirstObservation` episode at the settlement event, while flat
leaves `NoOpenEpisode`. Ambiguous settlement leaves attribution interrupted.

- [ ] Test ID collision, missing start, wrong account, per-market multiple
  fills, fill overrun, repeated fill, invalid account/process transitions,
  backstop on both known and unseen accounts, multiple unresolved causes,
  partial-liquidation same-event ordinal `0`/`1` separation followed by trade
  close, episode close/interruption, funding suppression, authoritative-trade
  recovery, settlement partial/full/ambiguous, partial-settlement same-event
  ordinal `0`/`1` separation followed by funding, fact-only settlement PnL,
  and proof that settlement never closes a liquidation process.
- [ ] Run full ledger/replay/strict gates and commit with
  `feat(state): reduce conservative liquidation state`.

---

### Task 7A: Add bounded block-delta validation context

**Files:**
- Modify: `crates/canonical-ledger/src/reducer.rs`
- Modify: `crates/canonical-ledger/src/ledger.rs`
- Modify: `crates/canonical-ledger/src/lib.rs`
- Modify: `crates/canonical-ledger/tests/atomic.rs`
- Create: `crates/canonical-ledger/tests/block_delta_validation.rs`

Add a backward-compatible default trait method:

```rust
pub struct BlockDeltaEntry<'a> {
    pub key: &'a StateKey,
    pub block_start_value: Option<&'a [u8]>,
    pub block_final_value: Option<&'a [u8]>,
    pub write_count: u32,
}

pub struct BlockDeltaView<'a> {
    entries: &'a [BlockDeltaEntry<'a>],
}

fn validate_block_delta(
    &self,
    final_state: &StateView<'_>,
    delta: &BlockDeltaView<'_>,
    context: &ApplyContext<'_>,
) -> Result<(), ReducerError> {
    self.validate_block(final_state, context)
}
```

The ledger builds entries in deterministic `StateKey` byte order. Repeated
writes in separate events collapse to one entry with the block-start previous
value, block-final current value, and checked write count; per-event duplicate
keys still fail earlier and are not hidden. Freeze production limits for
the normalized view:

```text
MAX_BLOCK_DELTA_ENTRIES = 1_000_000
MAX_BLOCK_DELTA_REFERENCED_BYTES = 256 * 1024 * 1024
```

Add both limits to `LedgerLimits`; custom limits must be nonzero and no larger
than the existing block work ceilings. Count each unique key once plus its
block-start and block-final values when present, using checked arithmetic,
before reserving the entry vector. Enforce the existing mutation-byte ceiling
independently because repeated writes are intentionally collapsed in the
normalized view. Every child reducer receives the same normalized view.

- [ ] Write red tests for deterministic order, put/update/delete, repeated
  writes, block-start/final values, write-count overflow, byte/entry limits,
  default compatibility, late delta-invariant failure, and full rollback.
- [ ] Run full ledger/replay/strict gates and commit with
  `feat(ledger): expose bounded block delta validation`.

---

### Task 7B: Add the production composite

**Files:**
- Create: `crates/canonical-ledger/src/composite.rs`
- Modify: `crates/canonical-ledger/src/lib.rs`
- Create: `crates/canonical-ledger/tests/composite_account_state.rs`

`CanonicalStateReducerV1` dispatches in fixed order:

```text
market -> order -> trade-v1-fact -> trade-v2-fact -> account-flow -> position
```

All components see the same pre-event state; later events see all prior
candidate mutations. Merge component mutations with explicit cross-component
key-collision denial. Freeze
`CanonicalStateReducerV1::VERSION =
"hyperliquid-alpha-desk-canonical-state@1.0.0"` with explicit component
versions including `CanonicalTradeReducerV2`. Restore rejects any checkpoint
whose reducer-set version differs. Composite block validation passes the same
`BlockDeltaView` to every child in fixed component order.
The frozen component set includes both `CanonicalTradeReducerV1 @1.0.0` and
`CanonicalTradeReducerV2 @2.0.0`; removing either is a reducer-set version
change.

- [ ] Test same-event pre-state visibility, cross-component collision, current
  overwrite across separate events, immutable collision, late reducer failure,
  late child delta-invariant failure, component version mismatch, V1
  checkpoint refusal, and full rollback.
- [ ] Prove one trade plus associated order lifecycle events applies each
  participant exactly once. Mixed legacy participant-free and enriched trades
  keep V1 facts while only enriched trades produce positions.
- [ ] Commit with `feat(state): compose canonical account state`.

---

### Task 8: Retain composite position replay evidence

**Files:**
- Modify: `tools/state-replay/src/account.rs`
- Modify: `tools/state-replay/src/lib.rs`
- Modify: `tools/state-replay/src/main.rs`
- Modify: `tools/state-replay/tests/account_e2e.rs`
- Modify: `tools/state-replay/tests/cli.rs`
- Modify: `justfile`
- Modify: `README.md`
- Modify: `docs/STATUS.md`
- Modify: `docs/contracts/deterministic-state-v1.md`
- Modify: `docs/runbooks/state-replay-evidence.md`
- Modify: `docs/superpowers/plans/2026-07-29-canonical-account-ledger.md`
- Modify: `docs/superpowers/plans/2026-07-30-canonical-trade-positions.md`

Tasks 1-7 must pass verification and independent code review. Account-plan
Task 6 then creates and commits the baseline account-flow/composite
`state-replay account-e2e` runner. This Task 8 extends that exact committed
baseline with position, episode, liquidation, and settlement scenarios; a
design review alone never unblocks evidence work.
Require repeated full replay, checkpoint resume across reversal and funding,
byte-identical state/full-receipt hashes, exact namespace counts, duplicate
denial, start-position mismatch rollback, unresolved backstop behavior, and
schema denial.

Evidence labels:

```text
evidence_class = synthetic_canonical_position
state_semantics = exact_trade_anchored_quantity_and_analytical_episode_flows
source_qualification = synthetic_unassessed
```

Only synthetic contract qualification may be true. Deployed source, protocol
entry-price parity, source closed-PnL, execution-fee attribution, opening
balance, margin, liquidation-price, and live-product qualification remain
false until separately proven.

## Production source/commit-join gate

The existing `map_node_v1_record` trade path emits `ProvisionalSource`; the
canonical ledger correctly rejects it. Before `hl-core` may apply enriched
node trades, implement and review a source join that binds the auxiliary trade
batch to an immutable committed block using exact chain, height, block time,
transaction identity/order, source hashes, and a qualified node/source
version. Missing, ambiguous, duplicate, or divergent joins quarantine the
block. This gate must retain a real recording and restart/replay evidence; it
may not promote a synthetic fixture or merely change the confirmation enum.

Until this gate passes, Tasks 1-8 are repo-ready synthetic contract proof, not
a production position path.

## Follow-on node-fill source gate

Before mapping node `Fills` into account state, retain a real node recording
and prove:

- how account identity is represented or joined;
- whether both trade sides are emitted;
- stability and uniqueness of `tid`, `oid`, hash, and block context;
- fee token and builder-fee denomination;
- closed-PnL sign and fee inclusion;
- TWAP identity behavior; and
- correlation with the same block's documented trade rows.

If the source omits account identity and no exact join key is proven, it
remains evidence-only. Never join by timestamp proximity.

## Verification

Each task runs its focused suite plus:

```bash
cargo +1.97.1 test -p domain-types -p api-contracts -p canonical-events --locked --offline
cargo +1.97.1 test -p canonical-ledger -p replay-engine --locked --offline
cargo +1.97.1 clippy -p domain-types -p api-contracts -p canonical-events -p canonical-ledger -p replay-engine --all-targets --all-features --locked --offline -- -D warnings
cargo +1.97.1 fmt --all -- --check
just generated
just deny
git diff --check
```

Use serialized `RUST_TEST_THREADS=1 just verify` for the full repository gate
until the known Stage Gate repository-mutation isolation race is separately
fixed.

## Decision log

- 2026-07-30: Task 1 completed at `4b44962` plus exact-API remediation
  `7bc7a41`. The final surface has signed exact-only `PositionQuantity`,
  bounded canonical `ExactQuoteNotional`, no public arbitrary-`BigInt`
  admission, and checked upward-only normalization. Parent and independent
  review passed 53 domain tests, strict Clippy, formatting, and diff checks.
- 2026-07-30: Task 2 completed at `c4335f5` plus canonical-CLOID remediation
  `5d890d3`. Enriched trades retain exact buyer/seller participant anchors,
  preserve participant-free V1 bytes, and keep node output provisional.
  Hyperliquid CLOIDs are accepted only as lowercase `0x` plus 32 lowercase
  hexadecimal digits at both wire and node-mapping boundaries. Parent and
  independent review passed the full domain/API/event suites, generated-drift
  proof, strict Clippy, formatting, and diff checks.
- 2026-07-30: Task 3 completed at `9b23b7c` plus persisted-version remediation
  `fd20805`. V1 facts remain byte-frozen; enriched trades add exact V2
  participant anchors and checked buyer-positive/seller-negative
  reconciliation through the separately versioned V2 reducer set. Mixed
  replay binds V2 producer/parser identities, refuses component checkpoints
  at both store and reducer-version boundaries, and keeps every qualification
  flag false. Parent and independent review passed full ledger/replay suites,
  eight literal persisted-record goldens, the V2 codec boundary matrix,
  strict Clippy, formatting, and diff checks.
- 2026-07-30: Starting from flat was rejected because node trade rows provide
  an exact `start_pos`; ignoring it would make retained-range position state
  false.
- 2026-07-30: One enriched cross-account trade owns both position effects.
  Order lifecycle fills do not mutate positions.
- 2026-07-30: Protocol-observed signed quantity and analytical entry/PnL are
  separate. Entry price is a frontend component and non-terminating VWAP is
  stored as exact notional/quantity components.
- 2026-07-30: Identity-less `FeeCharged` cannot populate a position or episode
  fee. Node fill mapping remains blocked until real source evidence proves an
  account identity or exact join key.
- 2026-07-30: Backstop and settlement remain conservative facts where the
  current payload omits side, price, cost basis, or process linkage.

## Rollback

- Clean pre-position fallback: `1d63bc5`.
- If participant enrichment cannot preserve prior V1 envelope compatibility,
  stop and introduce an explicitly versioned payload/namespace rather than
  reinterpret stored bytes.
- If a source field cannot be mapped exactly, preserve the raw observation,
  emit evidence-only disposition, and keep dependent state unresolved.
