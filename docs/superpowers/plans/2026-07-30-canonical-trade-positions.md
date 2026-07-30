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

- [x] Write red tests for first nonzero anchor, buyer/seller symmetry, long and
  short add/reduce/flat/reversal, mixed scales, overflow, missing participants,
  reordered identities, unresolved metadata, duplicate effect, start-position
  mismatch, seeded unresolved-state re-anchor, all four
  known-quantity/first-anchor combinations, corrupt current state, mixed
  legacy/enriched test-dispatcher application, and late block rollback.
- [x] Require exact current market metadata. Normalize price to the active
  price scale and fill/start/result quantities to the active quantity scale
  only by exact upward rescaling; then require price tick alignment and fill,
  both starts, and both results to be lot-aligned. Perform exact notional
  multiplication only after this normalization. The notional is
  validation-only in Task 4; `trade.v2` owns the source price and Task 5B owns
  analytical notional persistence.
- [x] Ensure order-fill events are unsupported by this reducer and cannot
  double count.
- [x] Add strict key-bound 16 KiB codecs and 64 KiB preallocation bounds;
  test the ledger's stricter 4 KiB mutation-key ceiling separately.
- [x] Use a test-only dispatcher that evaluates market,
  `CanonicalTradeReducerSetV2`, and `CanonicalPositionReducerV1` against the
  same pre-event state, skips position reduction for legacy trades, and
  rejects cross-child key collisions. Production mixed archive/checkpoint
  replay remains Task 7/8 under `CanonicalStateReducerV1::VERSION =
  "hyperliquid-alpha-desk-canonical-state@1.0.0"`.
- [x] Run ledger/replay tests and strict gates.
- [x] Commit with `feat(state): reconstruct anchored position quantity`.

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

pub enum EpisodeEffectKindV1 {
    Opened,
    Updated,
    Closed,
    Interrupted,
}

pub struct PositionEpisodeRecordV1 {
    episode_id: PositionEpisodeId,
    account_id: Address,
    market_id: MarketId,
    opening_anchor_event_id: EventId,
    opening_leg_ordinal: u8,
    opening_position: PositionQuantity,
    close_event_id: Option<EventId>,
    close_cause: Option<EpisodeCloseCauseV1>,
    completeness: EpisodeCompletenessV1,
    buy_quantity: Quantity,
    buy_notional: ExactQuoteNotional,
    sell_quantity: Quantity,
    sell_notional: ExactQuoteNotional,
    funding_paid: QuoteAmount,
    funding_received: QuoteAmount,
    status: EpisodeStatusV1,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

pub struct PositionEpisodeCurrentRecordV1 {
    account_id: Address,
    market_id: MarketId,
    episode_id: Option<PositionEpisodeId>,
    attribution_resolution: EpisodeAttributionResolutionV1,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

pub struct PositionEpisodeEffectFactRecordV1 {
    event_id: EventId,
    account_id: Address,
    market_id: MarketId,
    leg_ordinal: u8,
    episode_id: PositionEpisodeId,
    effect_kind: EpisodeEffectKindV1,
    buy_quantity_delta: Quantity,
    buy_notional_delta: ExactQuoteNotional,
    sell_quantity_delta: Quantity,
    sell_notional_delta: ExactQuoteNotional,
    funding_paid_delta: QuoteAmount,
    funding_received_delta: QuoteAmount,
    close_cause: Option<EpisodeCloseCauseV1>,
    rule_version: String,
}
```

Derive `PositionEpisodeId` with BLAKE3 derive-key context
`hyperliquid-alpha-desk/position-episode-id/v1`. Hash, in order, the raw
20-byte account, market UTF-8, and opening-event UTF-8, each framed as an
unsigned u64 big-endian length followed by the bytes, then hash the raw u8
opening leg ordinal. The ordinal is exactly `0` or `1`. Reject derivation when
the framed source identity exceeds the established 64 KiB bound. Encode the
result as `pos_ep_` followed by exactly 64 lowercase hexadecimal characters.
Episode decode recomputes this identity and rejects a mismatch. Freeze at
least one literal derived-ID vector plus independent perturbations of every
input.

The namespace/schema pairs are:

```text
position-episode.v1
  hyperliquid-alpha-desk/position-episode/v1
position-episode-current.v1
  hyperliquid-alpha-desk/position-episode-current/v1
position-episode-effect-fact.v1
  hyperliquid-alpha-desk/position-episode-effect-fact/v1
```

Keys are:

```text
position-episode.v1                  frame(episode_id)
position-episode-current.v1          frame(account) + frame(market)
position-episode-effect-fact.v1      frame(event_id) + frame(account) + frame(market) + frame(leg_ordinal)
```

Key frames use the established unsigned u64 big-endian length plus bytes.
The ordinal key component is therefore `frame([ordinal])`, including the
eight-byte length `1` followed by the single ordinal byte. Opening ordinals and
later effect ordinals are independent fields even when they happen to match.

Leg ordinal is bounded to `0..=1`: `0` is the first or only episode effect for
an account-market event, and `1` is the second residual/reseed effect when the
same event closes or interrupts one episode and opens another. Trade reversal,
partial liquidation, and partial settlement therefore use effect ordinal `0`
for the old leg and ordinal `1` for the new residual/reseeded leg. The exact
lowercase wire variants are
`complete_from_flat` / `partial_from_first_observation`,
`trade_flat` / `trade_reversal` / `liquidation_fill` / `settlement` /
`backstop_interrupted`, `open` / `closed` / `interrupted`, and
`no_open_episode` / `resolved` / `interrupted`, and
`opened` / `updated` / `closed` / `interrupted`.

An open episode has neither close field. Closed and interrupted episodes have
both close fields. `Closed` accepts only `TradeFlat` or `TradeReversal`.
`Interrupted` accepts only `LiquidationFill`, `Settlement`, or
`BackstopInterrupted`. Opening and closing event IDs may be equal for a
first-observation reversal. `CompleteFromFlat` requires a zero
`opening_position`; `PartialFromFirstObservation` requires a nonzero
`opening_position`.

All cumulative and effect-delta buy/sell quantities, exact notionals, and
funding paid/received values are nonnegative. For each buy or sell pair,
quantity is zero if and only if its notional is zero. An episode effect is a
delta, not a state snapshot. `Opened` and `Updated` have no close cause;
`Closed` and `Interrupted` follow the same close-cause matrix as the episode
record. Every effect carries rule version
`hyperliquid-alpha-desk-canonical-position-episode@1.0.0`.

The current-record codec enforces only the structural rule:
`Resolved` requires `Some(episode_id)`, while `NoOpenEpisode` and
`Interrupted` require `None`. Task 5A also exposes a read-only,
`StateView`-aware reference validator. A resolved pointer is valid only when
the target key exists, `PositionEpisodeRecordV1::decode_at` succeeds, account
and market match, and the target status is `Open`. Non-resolved pointers must
not supply a target. Task 5B calls this validator for every existing loaded
resolved pointer and validates every proposed episode/current pair in memory
before emitting it. Task 7A revalidates touched final pairs through bounded
`BlockDeltaView` before block acceptance. A current pointer never references a
closed or interrupted episode.

Fields remain private. Public APIs are limited to deterministic ID derivation,
`state_key`, `decode`, `decode_at`, the state-aware reference validator, and
read-only accessors. Validated construction and encoding remain
`pub(super)`. Strict key-bound codecs use the established 16 KiB value and
64 KiB preallocation rules. The production ledger's separate 4 KiB mutation
key ceiling remains independently enforced.

Task 5A freezes the immutable effect identity and codec. Task 5B owns stateful
insertion and must decode any existing effect at the target key before
rejecting it as `position_episode.effect_identity_collision`; identical bytes
are not an idempotent overwrite.

- [x] Write red codec/key tests for the literal deterministic ID, every ID
  input perturbation, both opening/effect ordinals, all literal
  schema/enum/record/key vectors, current pointers, completeness and opening
  position, derived-ID mismatch, the complete status/close-cause matrix,
  nonnegative totals/deltas, quantity/notional pairing, exact-notional
  256-byte/155-digit/512-bit/scale-76 bounds, key mismatch, canonical JSON,
  same-event two-effect key separation, state-aware missing/corrupt/
  key-mismatched/wrong-identity/non-open pointer targets, exact 16 KiB values,
  exact 64 KiB keys and each +1 rejection.
- [x] Implement records/codecs only; no transition logic.
- [x] Run focused/full ledger tests and strict gates.
- [x] Commit with `feat(state): add position episode records`.

---

### Task 5B: Reduce analytical episodes without canonical division

**Files:**
- Modify: `crates/canonical-ledger/src/position/episodes.rs`
- Modify: `crates/canonical-ledger/src/position/quantity.rs`
- Create: `crates/canonical-ledger/tests/position_episodes.rs`
- Modify: `crates/canonical-ledger/src/position/mod.rs`
- Modify: `crates/canonical-ledger/src/lib.rs`
- Create: `crates/replay-engine/tests/position_episodes.rs`
- Modify: `docs/contracts/deterministic-state-v1.md`

Add a distinct `CanonicalPositionEpisodeReducerV1` with:

```text
VERSION = hyperliquid-alpha-desk-canonical-position-episode@1.0.0
```

It supports only exact-schema enriched `TradeMatched`, `FundingPaid`, and
`FundingReceived`. Participant-free trades, `OrderFilled`, all fee events,
liquidation, backstop, and settlement remain unsupported until their named
reducers. Same-event V1/V2 trade facts and account-flow facts are sibling
outputs, never prerequisites.

Extract one crate-private validated-trade helper from Task 4 and use it from
both `CanonicalPositionReducerV1` and the episode reducer. Against the same
pre-event `StateView`, it loads exact market metadata, applies upward-only
price/fill/start normalization, enforces price tick and quantity lot alignment,
computes buyer/seller results with checked signed arithmetic, and validates the
full exact notional. Neither reducer may copy this arithmetic or read the
other's same-event mutations. The helper returns typed internal failures so
Task 4 preserves every existing reason code and byte-for-byte behavior while
the episode reducer maps its own frozen reason codes. Existing Task 4 focused
tests are mandatory refactor regressions.

The test dispatcher evaluates market events first. For other events it
evaluates, in fixed order, trade-set, account, quantity, and episode children
against the same pre-event state; concatenates their mutations in that order;
rejects cross-child duplicate keys; and runs child validation in fixed order.
Later events in the block see all candidate mutations from prior events.

For every touched account-market, freeze the pre-event consistency matrix:

```text
quantity current absent       <=> episode current absent
known quantity == zero        <=> NoOpenEpisode
known quantity != zero        <=> Resolved + matching key-bound Open episode
known quantity == None        <=> Interrupted
```

Any other pair, corrupt/key-mismatched record, orphan current, or resolved
closed/interrupted episode is `position_episode.current_pair_mismatch`.
Known quantities must normalize exactly to the source start position.
Unknown quantity may re-anchor from the next enriched trade's source start.
First observation is the both-absent case.

Task 5B preserves this invariant by induction: validate every loaded pair and
every proposed episode/current pair in memory before returning mutations. Do
not add an O(total-state) `validate_block` scan. Task 7A's bounded delta view
will validate both touched namespaces in both directions. The production
composite refuses all pre-episode component checkpoints and requires replay
under its new reducer-set version, so no implicit migration can introduce
orphan state.

For each normalized participant, buyer activity always increments buy
quantity/notional and seller activity always increments sell
quantity/notional. This remains true for entry, reduction, close, and both
reversal pieces. The transition table is:

```text
start == 0:
  open CompleteFromFlat leg 0 with the full fill delta

first observation or re-anchor with start != 0:
  create PartialFromFirstObservation leg 0 anchored at source start

start and result have the same nonzero sign:
  apply the full fill to the new/existing Open episode

result == 0:
  apply the full fill and close leg 0 with TradeFlat

start and result have opposite nonzero signs:
  close old/partial leg 0 with TradeReversal using only closed_quantity
  open CompleteFromFlat residual leg 1 from opening_position == 0
```

For reversal:

```text
closed_quantity   = min(checked_magnitude(start_position), fill_quantity)
residual_quantity = fill_quantity - closed_quantity
```

Never call unchecked `abs` on signed raw values. Branch by role/sign and use
checked negation for a buyer closing a short. The old/partial episode receives
only `closed_quantity` and `price * closed_quantity`; the new episode receives
only residual quantity and notional. Require checked quantity and notional
split conservation against the full fill. A first-observation reversal creates
a partial episode opened and closed by the same event at ordinal `0`, plus the
complete residual ordinal `1`.

Existing episode identity/opening fields remain immutable. Upward-align
cumulative buy/sell quantity scales before checked addition. Use
`ExactQuoteNotional::checked_product` for each attributed portion and
`checked_add` for cumulative notionals. Entry/exit VWAP remains the exact
`(notional, quantity)` pair; no canonical decimal division occurs.

Mutation order is exactly
`buyer(effect leg 0, effect leg 1, episode leg 0, episode leg 1, current)`
followed by the same sequence for seller, omitting nonexistent legs.
Decode/key-bind any existing immutable effect before rejecting
`position_episode.effect_identity_collision`.
Decode/key-bind a pre-existing newly derived episode before rejecting
`position_episode.episode_identity_collision`. Identical bytes are not an
idempotent overwrite. Only the validated existing Open episode may be
overwritten. Reject reducer-local duplicate mutation keys before returning;
stage both participants fully so a late seller failure returns no mutations.

Funding independently validates exact ordered envelope account/market
identity, positive payload amount, exact current market metadata, and the
prestate matrix. Suppression is byte-empty:

```text
both currents absent                 -> no episode mutations
known zero + NoOpenEpisode           -> no episode mutations
unknown quantity + Interrupted       -> no episode mutations
known nonzero + Resolved Open target -> attribute funding
```

Resolved attribution upward-aligns existing paid, received, and incoming
`QuoteAmount` values to their maximum scale, checked-adds exactly the named
paid or received side, emits ordinal-`0` `Updated` effect with zero trade
deltas and the one funding delta, updates episode provenance, and refreshes
the resolved current pointer. Any corrupt or inconsistent resolved state is an
error, never suppression. The account reducer retains its independent funding
flow/fact in the same event. `FeeCharged` stays out of episodes because V1 has
no execution identity; time proximity is forbidden.

Freeze these reducer reason codes and precedence after unsupported/identity
checks, then prerequisites, loaded-state validity, arithmetic, collisions, and
duplicate mutations:

```text
position_episode.unsupported_event
position_episode.identity_mismatch
position_episode.market_prerequisite_missing
position_episode.market_prerequisite_invalid
position_episode.market_prerequisite_unresolved
position_episode.quantity_current_invalid
position_episode.episode_current_invalid
position_episode.episode_reference_invalid
position_episode.current_pair_mismatch
position_episode.start_position_mismatch
position_episode.quantity_arithmetic
position_episode.notional_arithmetic
position_episode.funding_arithmetic
position_episode.effect_prior_invalid
position_episode.effect_identity_collision
position_episode.episode_prior_invalid
position_episode.episode_identity_collision
position_episode.duplicate_mutation_key
```

An existing effect or newly derived episode key is decoded first. Decode,
canonicality, or key-binding failure returns the corresponding
`*_prior_invalid`; only a valid existing immutable record returns
`*_identity_collision`. A current record codec failure returns its named
`*_current_invalid`; a missing/corrupt/key-mismatched/non-open resolved target
returns `episode_reference_invalid`; structurally valid cross-family pair
disagreement returns `current_pair_mismatch`.

For a `CompleteFromFlat` episode closed only by trades,
`observed_signed_trade_notional_delta = sell_notional - buy_notional`.
This is an analytical trade-only metric, not protocol/source `closedPnl`, and
excludes fees and funding. It is unavailable when liquidation, settlement,
backstop, or another non-trade cause changes quantity. Expose:

```rust
pub fn observed_signed_trade_notional_delta(
    &self,
) -> Result<Option<ExactQuoteNotional>, PositionStateError>
```

Return `Some(sell - buy)` only for `CompleteFromFlat` episodes whose status is
`Closed` and cause is `TradeFlat` or `TradeReversal`; use checked subtraction.
Return `None` for partial, open, or interrupted episodes.

- [x] Test buyer/seller flat-open/add/reduce/flat/reversal; first partial
  same-side/reduce/flat/reversal; unresolved re-anchor from flat/nonzero;
  both reversal ordinals/IDs; split quantity/notional conservation;
  non-terminating VWAP pairs; mixed-scale increases; tick/lot/downscale,
  checked-magnitude, signed quantity, exact notional, cumulative quantity, and
  cumulative notional overflow.
- [x] Test every invalid quantity/episode prestate pair, corrupt/key-mismatched
  records, closed immutability, state-aware reference validation, start
  mismatch, effect/new-episode collision after decoding corrupt,
  key-mismatched, and identical prior bytes, and reducer-local/cross-child
  duplicate keys.
- [x] Test paid/received funding attribution, upward scale alignment, all three
  byte-empty suppression states, stale flat/nonzero mismatch, exact market and
  ordered identity failures, and proof that account funding flow remains a
  sibling while fees are unsupported.
- [x] Test same-event prestate behavior, later same-block visibility, buyer
  success plus seller failure rollback, late child failure rollback, mixed
  legacy/enriched ownership, checkpoint refusal, and repeated/resumed replay
  byte identity under the test reducer set.
- [x] Document exact analytical-only metric availability and the missing fee
  execution identity.
- [x] Run ledger/replay tests and strict gates.
- [x] Commit with `feat(state): add exact analytical position episodes`.

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

Textual liquidation, event, and market identities use their exact UTF-8 bytes;
account identities use the raw 20 address bytes. Freeze the six record schemas:

```text
hyperliquid-alpha-desk/liquidation-current/v1
hyperliquid-alpha-desk/liquidation-start-fact/v1
hyperliquid-alpha-desk/liquidation-fill-fact/v1
hyperliquid-alpha-desk/liquidation-market-flow-current/v1
hyperliquid-alpha-desk/backstop-liquidation-fact/v1
hyperliquid-alpha-desk/position-settlement-fact/v1
```

Freeze the reducer/rule version as
`hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0`.
`CanonicalLiquidationReducerV1::VERSION` supplies that exact value. Task 6A
adds the typed records and version marker; Task 6B adds `EventReducer`
behavior.

`LiquidationCurrentRecordV1` binds the started account, margin observation,
start/backstop/last-observation provenance, and an observed process status with
exact lowercase wire variants `started` and `backstop_observed`. No V1 payload
proves completion, so there is no `completed`, `terminal`, or final-fill
status. Market quantity aggregates are always separate per liquidation,
account, and market; never sum across markets.
Backstop facts retain both accounts and the explicitly missing price/basis.
Settlement facts have no liquidation ID and therefore no process status.

```rust
pub enum LiquidationObservedStatusV1 {
    Started,
    BackstopObserved,
}

pub enum LiquidationSourceValueResolutionV1 {
    UnavailableFromSource,
}
```

The exact wire variants are `started` and `backstop_observed`. Neither means
that the liquidation is complete or terminal. The only V1 source-value
resolution wire variant is `unavailable_from_source`.

Freeze the records as source-observation facts, not inferred accounting:

```rust
pub struct LiquidationCurrentRecordV1 {
    liquidation_id: LiquidationId,
    account_id: Address,
    start_margin_value: UsdAmount,
    start_maintenance_requirement: UsdAmount,
    observed_status: LiquidationObservedStatusV1,
    start_event_id: EventId,
    start_block_height: BlockHeight,
    start_transaction_index: u32,
    start_canonical_event_index: u32,
    first_backstop_event_id: Option<EventId>,
    first_backstop_block_height: Option<BlockHeight>,
    first_backstop_transaction_index: Option<u32>,
    first_backstop_canonical_event_index: Option<u32>,
    last_observation_event_id: EventId,
    last_observation_block_height: BlockHeight,
    last_observation_transaction_index: u32,
    last_observation_canonical_event_index: u32,
    rule_version: String,
}

pub struct LiquidationStartFactRecordV1 {
    liquidation_id: LiquidationId,
    event_id: EventId,
    account_id: Address,
    margin_value: UsdAmount,
    maintenance_requirement: UsdAmount,
    block_height: BlockHeight,
    transaction_index: u32,
    canonical_event_index: u32,
    payload_blake3: [u8; 32],
    rule_version: String,
}

pub struct LiquidationFillFactRecordV1 {
    liquidation_id: LiquidationId,
    event_id: EventId,
    account_id: Address,
    market_id: MarketId,
    price: Price,
    quantity: Quantity,
    block_height: BlockHeight,
    transaction_index: u32,
    canonical_event_index: u32,
    payload_blake3: [u8; 32],
    rule_version: String,
}

pub struct LiquidationMarketFlowCurrentRecordV1 {
    liquidation_id: LiquidationId,
    account_id: Address,
    market_id: MarketId,
    observed_filled_quantity: Quantity,
    first_fill_event_id: EventId,
    first_fill_block_height: BlockHeight,
    first_fill_transaction_index: u32,
    first_fill_canonical_event_index: u32,
    last_fill_event_id: EventId,
    last_fill_block_height: BlockHeight,
    last_fill_transaction_index: u32,
    last_fill_canonical_event_index: u32,
    rule_version: String,
}

pub struct BackstopLiquidationFactRecordV1 {
    liquidation_id: LiquidationId,
    event_id: EventId,
    account_id: Address,
    backstop_account_id: Address,
    market_id: MarketId,
    quantity: Quantity,
    transfer_price_resolution: LiquidationSourceValueResolutionV1,
    entry_price_resolution: LiquidationSourceValueResolutionV1,
    block_height: BlockHeight,
    transaction_index: u32,
    canonical_event_index: u32,
    payload_blake3: [u8; 32],
    rule_version: String,
}

pub struct PositionSettlementFactRecordV1 {
    event_id: EventId,
    account_id: Address,
    market_id: MarketId,
    settlement_price: Price,
    settled_quantity: Quantity,
    realized_pnl: QuoteAmount,
    block_height: BlockHeight,
    transaction_index: u32,
    canonical_event_index: u32,
    payload_blake3: [u8; 32],
    rule_version: String,
}
```

The start/current margin values are nonnegative, share one scale, and preserve
`margin_value < maintenance_requirement`. Fact decimals retain source scale;
there is no scale relation across price, quantity, settlement price, or PnL.
Fill price and fill quantity are strictly positive. Per-market
`observed_filled_quantity` is the unsigned exact sum of source fill quantities,
not a signed position delta or remaining quantity. Task 6B additions align
existing and incoming quantities upward to their maximum scale, checked, with
no downscale or rounding. Backstop accounts differ and quantity is strictly
positive. Both backstop resolution fields must be
`UnavailableFromSource`; the record exposes no known transfer price or entry
basis. Settlement price is nonnegative, settled quantity is strictly positive,
and realized PnL remains signed.

Define event position as the lexicographically ordered tuple
`(block_height, transaction_index, canonical_event_index)`. Current and
aggregate first/last provenance must bind tuple to event identity: equal event
IDs require equal tuples, while distinct event IDs require strict tuple order.
Distinct events may share one block but never the complete tuple. `Started`
requires all four first-backstop fields to be `None`. `BackstopObserved`
requires all four to be `Some`, a distinct first-backstop event, and:

```text
start_position < first_backstop_position <= last_observation_position
```

Equality between first-backstop and last-observation positions is valid only
when their event IDs are equal; otherwise their order is strict. The same
identity/tuple rule applies to first-fill and last-fill provenance.

Every retained fill updates current `last_observation_*` without changing
status. The first retained backstop sets `first_backstop_*`; later backstops
preserve the first provenance and update only `last_observation_*`.
`LiquidationMarketFlowCurrentRecordV1` similarly preserves first-fill
provenance and advances last-fill provenance. All records require the exact
frozen rule version.

Every immutable fact copies `CanonicalEventEnvelope::payload_hash()` exactly.
The field-ordered JSON wire name is `payload_blake3`, encoded as exactly 64
lowercase hexadecimal characters. Decoders reject wrong length, uppercase, or
non-hex values; Task 6B constructs and validates the envelope-to-fact binding.
Current and aggregate records have no single payload hash.

Immutable fact validity is standalone: its codec does not require that the
current/start sibling already exists. Reducer-level sibling and envelope
checks are Task 6B and remain block atomic. Prior immutable facts are decoded
and key-bound first: malformed/noncanonical/key-mismatched bytes are
`prior_invalid`; any valid existing fact at the same key, including identical
bytes, is `identity_collision`. Whole-block duplicate delivery idempotence is
a separate ledger contract. No record exposes `complete`, terminality, side, a
known transfer price, a known entry/cost basis, or a
settlement-to-liquidation link.

V1 treats the source `LiquidationId` as a globally unique process identity.
The current key therefore remains `frame(liquidation_id)` exactly as frozen;
reuse for another started account is an identity collision, never a second
process. This is enforced fail-closed and does not depend on present source
qualification. Once a current exists, every later `LiquidationStarted` for
that ID is `process_identity_collision`, including the same account and valid
or byte-identical content; only whole-block ledger redelivery is idempotent,
and no later start fact/current mutation commits. A fill/backstop before a
retained start and a mismatched liquidated account are Task 6B transition
errors. Because
`BackstopObserved` is explicitly nonterminal, later same-account fills and
strictly later repeated backstop observations are accepted; exact event-key
reuse collides, and provenance regression fails. The immutable fact shapes
retain enough identity to diagnose failures without weakening the current key.

Freeze exact envelope bindings for Task 6B:

```text
LiquidationStarted   accounts [account]            markets []
LiquidationFill      accounts [account]            markets [market]
BackstopLiquidation  accounts [account, backstop]  markets [market]
PositionSettled      accounts [account]            markets [market]
```

The backstop account order is literal. `PositionSettled` remains independent:
it requires no liquidation start/current and never mutates liquidation process
status.

Keys use unsigned 64-bit big-endian length framing, checked/preallocated to at
most 64 KiB. Values are canonical, unknown/duplicate-field-denying JSON of at
most 16 KiB. Constructors validate private fields; `decode_at` recomputes the
exact key. The later production `StateImage` retains its separate 4 KiB key
limit.

- [x] Write red codec/key tests for every record, process status, account
  binding, per-market separation, duplicates, canonical JSON, key mismatch,
  16 KiB values, and 64 KiB keys.
- [x] Implement records only, run full ledger/replay/strict gates, and commit
  with `feat(state): add liquidation fact records`.

---

### Task 6B: Reduce liquidation, backstop, and settlement effects

**Files:**
- Modify: `crates/canonical-ledger/src/position/liquidations.rs`
- Create: `crates/canonical-ledger/tests/liquidation_state.rs`
- Modify: `crates/canonical-ledger/src/position/mod.rs`
- Modify: `crates/canonical-ledger/src/position/episodes.rs`
- Modify: `docs/contracts/deterministic-state-v1.md`

Freeze `CanonicalLiquidationReducerV1` as the sole atomic owner of
`LiquidationStarted`, `LiquidationFill`, `BackstopLiquidation`, and
`PositionSettled` for schema `1.0.0`. It returns one mutation vector built
entirely from the event's pre-state view. It does not invoke the trade-only
quantity or episode reducers against partially staged state.

No Task 6B event requires a `MarketCurrentRecordV1` prerequisite. These are
already-canonical direct source observations and must not be discarded because
metadata arrived later. Facts and liquidation flow retain source decimal
scales. Position arithmetic aligns the known signed position and incoming
unsigned quantity upward to their maximum scale with checked exact rescaling;
it never downscales or rounds. Classify results by the sign of the checked
candidate; never compute an absolute signed magnitude. Therefore
`i128::MIN + positive_fill` is a valid exact partial short reduction when the
checked result remains negative. Rescaling, addition, subtraction, and
aggregate overflow are deterministic arithmetic errors.

Freeze the valid quantity/episode pre-state pairs:

```text
quantity absent                    + episode-current absent
known zero + anchor                + NoOpenEpisode
known nonzero + anchor             + Resolved(open referenced episode)
known None + optional prior anchor + Interrupted
```

Every other cross-family combination, corrupt/key-mismatched current, or
invalid episode reference is rejected before any effect is returned.
Non-trade transitions preserve an existing `first_anchor_event_id`; an absent
pair first written as unresolved uses `None`. Every written quantity current
advances `last_event_id` and `last_block_height`.

`LiquidationFill` records its immutable explicit-price quantity and may reduce
known position quantity only when the exact current sign makes direction
unambiguous. Position arithmetic is:

```text
known long q > 0:  candidate = q - fill
known short q < 0: candidate = q + fill
```

A same-sign nonzero candidate is an exact partial reduction. Zero is exact
flat. An opposite-sign candidate, or any fill against known zero, is
`liquidation_state.fill_overrun` and rejects the whole event; V1 never converts
a liquidation fill into a reversal. An absent or already-unknown position does
not establish direction, so the source fact, process observation, and
per-market flow remain admissible while the position is created/refreshed as
`known_quantity = None` and attribution `Interrupted`.

Every applied non-trade quantity mutation interrupts the active episode, even
when a same-side nonzero quantity remains. Flat leaves `NoOpenEpisode`; a known
nonzero result opens a new `PartialFromFirstObservation` episode anchored at
the liquidation-fill event; an ambiguous result leaves attribution
`Interrupted`. It never manufactures entry basis.

`LiquidationStarted` requires no retained position. After exact envelope
binding, an invalid existing current is `process_prior_invalid`; any valid
existing current for the globally unique liquidation ID is
`process_identity_collision`, including a byte-identical or same-account
repeat. Otherwise it emits the start fact followed by a `Started` current.

`LiquidationFill` and `BackstopLiquidation` require a valid retained process,
the exact liquidated account, and an event position strictly greater than the
retained last observation. Fills are accepted in both `Started` and
`BackstopObserved`; status and first-backstop provenance are preserved while
last-observation provenance advances. Each fill updates only its exact
`(liquidation, account, market)` flow. Existing and incoming flow quantities
are aligned upward to their maximum scale and added exactly. Backstop quantity
never enters this fill aggregate.

`BackstopLiquidation` records both accounts, writes an independent unresolved
cause for each account/market even when no prior current record exists, sets
`known_quantity = None`, and marks the episode pointer interrupted so later
funding cannot attach to stale state. The next enriched trade may re-anchor
from its source `start_pos`; unresolved facts remain immutable history.

The first backstop sets `BackstopObserved` and first-backstop provenance.
Strictly later repeated backstops preserve that first provenance and update
only the last observation. The reducer handles accounts in literal envelope
order `[liquidated, backstop]`. For each it emits an independent unresolved
cause fact, preserves any prior quantity anchor, writes unknown quantity, and
interrupts a resolved episode. If preparation for either account fails, no
mutation for either account or the process/fact is returned.

`PositionSettled` remains independent from liquidation process state. When
current quantity is known and its sign makes reduction unambiguous, reduce by
`settled_quantity` toward zero and retain the explicit source `realized_pnl`
only in the immutable settlement fact. Settlement uses the same toward-zero
candidate arithmetic, but unlike a liquidation fill, absent, unknown,
known-zero, or overrun state is an admitted fact-only ambiguity: write
`known_quantity = None` and attribution `Interrupted`. It never reads or
writes liquidation current or liquidation flow.

Settlement price may be zero. `realized_pnl` remains signed, creates no account
quote-flow, and appears only in the settlement fact. Every exact settlement
interrupts the active episode; a known nonzero result opens a new
`PartialFromFirstObservation` episode at the settlement event, while flat
leaves `NoOpenEpisode`. Ambiguous settlement leaves attribution interrupted.

Freeze episode behavior for every non-trade transition:

```text
resolved old + exact nonzero result:
  old episode/effect ordinal 0 -> Interrupted with event cause
  new episode/effect ordinal 1 -> Open, PartialFromFirstObservation,
                                  opening_position = exact result
  current -> Resolved(new)

resolved old + exact flat:
  old episode/effect ordinal 0 -> Interrupted with event cause
  no new episode
  current -> NoOpenEpisode

resolved old + ambiguous result:
  old episode/effect ordinal 0 -> Interrupted with event cause
  no new episode
  current -> Interrupted

absent, NoOpenEpisode, or already Interrupted + ambiguous result:
  no fabricated episode or effect
  current -> Interrupted
```

The causes are `LiquidationFill`, `Settlement`, and `BackstopInterrupted`.
These events never set an episode to `Closed`; exact flat changes the pointer
to `NoOpenEpisode` while the prior episode remains `Interrupted`. All
non-trade episode effects have zero buy/sell quantity, notional, and funding
deltas. Existing cumulative trade/funding totals are unchanged. A new partial
episode starts with zero cumulative totals, so source liquidation price,
quantity, settlement price, and PnL remain exclusively in their immutable
facts. Backstop never creates ordinal `1`.

Zero encodings are canonical and hash-relevant. Fill and settlement transition
effects use buy/sell zero quantities at the maximum scale used for the
candidate arithmetic; backstop effects use the source backstop-quantity scale.
New partial episodes use buy/sell zero totals at the exact result-position
scale. Exact-flat `PositionQuantity` also retains the transition maximum scale.
All non-trade funding zero deltas and new funding totals use scale `0`.
All zero notionals use the canonical `ExactQuoteNotional` string `0`.

Freeze deterministic mutation order:

```text
primary immutable fact
liquidation market-flow current                 # fill only
liquidation process current                     # start/fill/backstop only
for each affected account in envelope order:
  unresolved-cause fact                         # backstop only
  episode effects ordinal 0 then ordinal 1
  old episode record then new episode record
  position-quantity current
  position-episode current
```

Settlement is `settlement fact -> account transition bundle`; start is
`start fact -> process current`. Before return, validate every proposed
quantity/episode pair and reject duplicate keys across the complete vector.

Freeze exact reason codes:

```text
liquidation_state.unsupported_event
liquidation_state.identity_mismatch
liquidation_state.start_fact_prior_invalid
liquidation_state.start_fact_identity_collision
liquidation_state.fill_fact_prior_invalid
liquidation_state.fill_fact_identity_collision
liquidation_state.backstop_fact_prior_invalid
liquidation_state.backstop_fact_identity_collision
liquidation_state.settlement_fact_prior_invalid
liquidation_state.settlement_fact_identity_collision
liquidation_state.process_prior_invalid
liquidation_state.process_missing
liquidation_state.process_identity_collision
liquidation_state.process_account_mismatch
liquidation_state.process_provenance_regression
liquidation_state.process_transition_invalid
liquidation_state.flow_prior_invalid
liquidation_state.quantity_current_invalid
liquidation_state.episode_current_invalid
liquidation_state.episode_reference_invalid
liquidation_state.current_pair_mismatch
liquidation_state.fill_overrun
liquidation_state.quantity_arithmetic
liquidation_state.flow_arithmetic
liquidation_state.unresolved_prior_invalid
liquidation_state.unresolved_identity_collision
liquidation_state.episode_effect_prior_invalid
liquidation_state.episode_effect_identity_collision
liquidation_state.episode_prior_invalid
liquidation_state.episode_identity_collision
liquidation_state.proposed_pair_invalid
liquidation_state.duplicate_mutation_key
```

Freeze total error precedence:

```text
unsupported/schema
envelope identity
primary immutable prior decode/key binding, then collision
process decode, existence, account, provenance, then transition
flow decode/key binding
affected accounts in envelope order:
  quantity-current decode/key binding
  episode-current decode/key binding
  episode reference decode/key binding
  current-pair validation
quantity arithmetic, then flow arithmetic, then proposed-pair validation
secondary identities in mutation/account order:
  unresolved fact
  ordinal-0 episode effect
  ordinal-1 episode effect
  new episode
  at each key: decode/key binding before valid identity collision
duplicate mutation keys
```

For repeated starts, process validation intentionally precedes start-fact
collision so a retained process always yields `process_identity_collision`.
For fill, backstop, and settlement, primary fact collision precedes provenance
or position-state checks. Identical fact bytes still collide; only ledger-level
whole-block `AlreadyApplied` is idempotent.

Same-block later fills and backstops observe earlier candidate state and must
advance the complete event-position tuple. A late failure rolls back the
entire event and therefore the block. Fresh replay, repeated replay, and
checkpoint-resumed replay must produce byte-identical state and hashes.
Checkpoint restore requires the exact reducer-set version; no version
substitution or fallback is permitted.

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

The episode child uses this bounded delta to validate both touched
`position-quantity-current.v1` and `position-episode-current.v1` namespaces in
both directions. It rejects orphan touched records and rechecks the
known-nonzero/resolved, known-zero/no-open, and unknown/interrupted matrix
against the final candidate state without scanning untouched global state.

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
market -> order -> trade-v1-fact -> trade-v2-fact -> account-flow
  -> position-quantity -> position-episode -> position-liquidation
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
change. Position quantity, analytical episode, and liquidation/settlement are
three separately versioned children; none may be collapsed into or silently
substituted for another.

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
- 2026-07-30: Task 4 contract was corrected at `e555ee0`, implemented at
  `e9b2195`, and completed with deterministic-compatibility remediation
  `9a45701`. Exact market-gated source anchors now reconstruct signed
  buyer/seller position quantity without assuming flat opening state, retain
  first/continued/reanchored transitions, and fail closed on scale,
  alignment, arithmetic, prerequisite, collision, or current-state errors.
  Parent and independent review passed 21 focused position tests, full
  ledger/replay suites, literal wire/key vectors, inclusive codec/key
  boundaries, strict Clippy, formatting, and diff checks. Qualification
  remains synthetic and the production composite/source join remains open.
- 2026-07-30: Task 5A entered design HOLD before tests because the original
  text named an immutable episode-effect namespace without a record and left
  deterministic episode-ID bytes unspecified. The corrected contract freezes
  a delta effect fact, BLAKE3 identity derivation, private record surfaces,
  opening-position and mutation provenance, status/cause matrices, exact
  numeric invariants, and structural versus state-aware current-pointer
  validation. Independent re-review returned GO before implementation.
- 2026-07-30: Task 5A completed at `a1372bd`. Strict episode, current-pointer,
  and immutable effect-delta records now freeze exact BLAKE3 identities,
  literal wire/key compatibility, opening and mutation provenance,
  status/cause matrices, exact notional/value/key bounds, and state-aware
  open-reference validation. Parent gates passed 9 focused record tests,
  3 internal reference tests, full ledger/replay suites, strict Clippy,
  formatting, and diff checks. Independent review returned GO after literal
  key, cross-family negative, independent 513-bit, wrong-key, and fixed
  episode-key remediation. Transition logic remains Task 5B.
- 2026-07-30: Task 5B entered design HOLD before RED. Two independent
  preflights found the original text did not freeze reducer ownership,
  same-prestate composition, quantity/episode pairing, shared normalized
  trade arithmetic, reversal deltas, funding suppression, collision ordering,
  bounded final validation, replay compatibility, or reason codes. The
  corrected contract now defines a separate episode reducer, a shared Task 4
  validation kernel, exact transition/funding tables, inductive validation
  until Task 7A's bounded delta audit, and explicit composite children.
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
