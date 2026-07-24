# Stage 2 Deterministic State Reconstruction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reconstruct exact, block-atomic Hyperliquid market, account, order, position, margin, liquidation, and order-book state from the canonical archive and live stream, with deterministic hashes, checkpoints, and continuous reconciliation.

**Architecture:** The synchronous `canonical-ledger`, `margin-models`, and `orderbook` crates implement pure reducers over typed state. `hl-core` surrounds them with asynchronous JetStream consumption, RocksDB write batches, checkpoints, state-delta publication, and health suppression. Protocol representation and analytical position episodes are separate but generated from the same committed event order.

**Tech Stack:** Rust 1.97.1, RocksDB 11.1.x behind `storage-ports`, Prost contracts, NATS JetStream pull consumers, BLAKE3 state hashing, Proptest, Loom, cargo-fuzz, Hyperliquid official state/API only for differential reconciliation, systemd.

## Global Constraints

- Stage 1 tag `stage-1-truth` and its gate record must verify before this plan begins.
- Global block order is serial. Pure preparation may be parallel, but committed effects are applied in deterministic partition order.
- A committed block is visible only after every event, invariant, state write, idempotency entry, and block checkpoint commits atomically.
- Duplicate event delivery has no additional effect.
- Protocol accounting is exact and separate from analytical episodes.
- Fixed-point arithmetic is checked; overflow is a critical data incident.
- Market metadata is versioned and applied before events that depend on the new version.
- Unknown or unsupported account/margin modes are explicit estimated/unsupported states, never silently treated as standard cross margin.
- A mismatched order book suppresses execution-cost, capacity, and fragility outputs until resynchronization.
- RocksDB is rebuildable from archive; archive plus compatible checkpoint is the recovery path.
- V1 remains read-only and contains no execution path.
- Every task follows TDD and ends in a focused commit.

---

### Task 1: Define canonical state, delta, and persistence ports

**Files:**
- Modify: `crates/storage-ports/src/lib.rs`
- Create: `crates/storage-ports/src/state.rs`
- Create: `crates/storage-ports/src/checkpoint.rs`
- Modify: `crates/canonical-ledger/src/lib.rs`
- Create: `crates/canonical-ledger/src/state.rs`
- Create: `crates/canonical-ledger/src/delta.rs`
- Create: `crates/canonical-ledger/src/errors.rs`
- Create: `crates/canonical-ledger/tests/state_contract.rs`
- Create: `schemas/proto/state/v1/state.proto`
- Create: `tools/state-vector-reference/Cargo.toml`
- Create: `tools/state-vector-reference/src/main.rs`
- Generate: `fixtures/golden/expected/empty-state.blake3`

**Interfaces:**
- Consumes: canonical event V1 contracts, exact values, and block ordering.
- Produces: `CanonicalState`, `StateDelta`, `ApplyContext`, `StateStore`, `CheckpointStore`, and stable state serialization used by reducers, replay, APIs, and features.

- [ ] **Step 1: Verify Stage 1**

```bash
git verify-tag stage-1-truth
just stage-1-gate
```

Expected: PASS.

- [ ] **Step 2: Write state determinism contract tests**

```rust
#[test]
fn empty_state_serialization_is_stable() {
    let state = CanonicalState::empty(ChainId::new("mainnet").unwrap());
    let expected = include_str!("../../../fixtures/golden/expected/empty-state.blake3").trim();
    assert_eq!(state.schema_version(), "1.0.0");
    assert_eq!(state.stable_hash().to_string(), expected);
}

#[test]
fn delta_order_is_stable_by_scope_and_key() {
    let mut delta = StateDelta::new(BlockHeight::new(42));
    delta.push(account_change("0xbb"));
    delta.push(account_change("0xaa"));
    delta.canonicalize_order();
    assert_eq!(delta.keys(), ["account:0xaa", "account:0xbb"]);
}
```

Implement `state-vector-reference` without depending on `canonical-ledger`. It serializes the exact empty-state canonical byte sequence documented in `crates/canonical-ledger/src/state.rs`, computes BLAKE3, and writes a lowercase 64-character digest followed by one newline. Run both implementations and require equality before checking in the vector:

```bash
cargo run -p state-vector-reference -- --chain mainnet --output fixtures/golden/expected/empty-state.blake3
EXPECTED="$(cat fixtures/golden/expected/empty-state.blake3)"
ACTUAL="$(cargo test -p canonical-ledger print_empty_state_hash -- --ignored --nocapture | tail -n 1)"
test "$EXPECTED" = "$ACTUAL"
```

The reference tool has its own serializer and may depend only on `blake3`, `clap`, and the Rust standard library.

- [ ] **Step 3: Define canonical in-memory state boundaries**

```rust
pub struct StateWatermarks {
    pub canonical_block: BlockHeight,
    pub market_registry_block: BlockHeight,
    pub account_state_block: BlockHeight,
    pub order_book_block: BlockHeight,
}

pub enum StateChange {
    Market { market_id: MarketId, state_hash: [u8; 32] },
    Account { account_id: AccountId, state_hash: [u8; 32] },
    Order { order_key: OrderKey, state_hash: [u8; 32] },
    Book { market_id: MarketId, state_hash: [u8; 32] },
    Reconciliation { scope: String, assessment_hash: [u8; 32] },
}

pub struct StateDelta {
    pub block_height: BlockHeight,
    pub block_time: ProtocolTime,
    pub applied_event_ids: Vec<EventId>,
    pub changes: Vec<StateChange>,
    pub post_state_hash: [u8; 32],
}

pub struct CanonicalState {
    pub chain_id: ChainId,
    pub applied_block: BlockHeight,
    pub markets: MarketRegistry,
    pub accounts: BTreeMap<AccountId, AccountState>,
    pub orders: BTreeMap<OrderKey, OrderState>,
    pub books: BTreeMap<MarketId, OrderBookState>,
    pub watermarks: StateWatermarks,
}

pub struct ApplyContext {
    pub block_height: BlockHeight,
    pub block_time: ProtocolTime,
    pub schema_version: String,
    pub confirmation_class: ConfirmationClass,
}
```

Use `BTreeMap` or canonical sorted iteration for hash generation; performance stores may use indexed representations internally but must expose stable iteration.

- [ ] **Step 4: Define persistence ports without RocksDB types**

```rust
#[derive(Debug, thiserror::Error)]
pub enum StateStoreError {
    #[error("state storage I/O failed: {0}")]
    Io(String),
    #[error("state codec version {0} is unsupported")]
    UnsupportedCodec(u32),
    #[error("event identity collision for {0}")]
    EventIdentityCollision(EventId),
    #[error("block {attempted} is behind checkpoint {checkpoint}")]
    BlockRegression { attempted: BlockHeight, checkpoint: BlockHeight },
    #[error("state batch was already committed")]
    AlreadyCommitted,
    #[error("state data is corrupt: {0}")]
    Corrupt(String),
}

pub trait StateStore: Send + Sync {
    type Batch: StateWriteBatch;
    fn begin_block(&self, block: BlockHeight) -> Result<Self::Batch, StateStoreError>;
    fn read_account(&self, id: &AccountId) -> Result<Option<AccountState>, StateStoreError>;
    fn read_market(&self, id: &MarketId) -> Result<Option<MarketState>, StateStoreError>;
    fn read_checkpoint(&self) -> Result<Option<BlockCheckpoint>, StateStoreError>;
}

pub trait StateWriteBatch {
    fn put_delta(&mut self, delta: &StateDelta) -> Result<(), StateStoreError>;
    fn mark_event_applied(&mut self, event_id: &EventId, payload_hash: &[u8; 32]) -> Result<(), StateStoreError>;
    fn set_checkpoint(&mut self, checkpoint: &BlockCheckpoint) -> Result<(), StateStoreError>;
    fn commit(self) -> Result<(), StateStoreError>;
}
```

- [ ] **Step 5: Add wire contracts and verify**

State deltas use Protobuf with decimal strings and stable enum values. Run:

```bash
cargo test -p canonical-ledger -p storage-ports
cargo run -p schema-check -- check schemas/proto/baseline/v1.pb target/schema/current.pb
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/storage-ports crates/canonical-ledger schemas/proto/state tools/state-vector-reference fixtures/golden/expected/empty-state.blake3 Cargo.toml Cargo.lock
git commit -m "feat(state): define deterministic state and persistence contracts"
```

---

### Task 2: Implement the RocksDB state backend and atomic idempotency

**Files:**
- Create: `services/hl-core/src/storage/mod.rs`
- Create: `services/hl-core/src/storage/rocks.rs`
- Create: `services/hl-core/src/storage/codec.rs`
- Create: `services/hl-core/src/storage/config.rs`
- Create: `services/hl-core/tests/rocks_atomicity.rs`
- Create: `config/rocksdb/default.toml`
- Create: `tools/state-inspect/src/main.rs`
- Create: `docs/storage/rocksdb.md`

**Interfaces:**
- Consumes: `StateStore`, state/delta wire contracts, block and event IDs.
- Produces: RocksDB column families, versioned key/value codecs, atomic block write batches, duplicate detection, checkpoints, and inspection tools.

- [ ] **Step 1: Write crash and duplicate tests**

Test crash points before batch write, during prepared serialization, after RocksDB commit, and before JetStream acknowledgement. On restart, the same block must either be entirely absent or entirely present; re-delivery must not alter the state hash.

- [ ] **Step 2: Create exact column families and key formats**

Open these column families: `meta`, `block_checkpoints`, `market_registry`, `account_state`, `position_state`, `order_state`, `book_state`, `rolling_windows`, `feature_online_state`, `provisional_state`, `idempotency`, and `reconciliation`.

Keys are version-prefixed and big-endian sortable:

The binary key codec uses these exact prefix bytes and length-delimited components:

```text
account key:    0x01 0x01 || address[20] || scope_len:u16_be || scope_utf8
order key:      0x01 0x02 || market_len:u16_be || market_utf8 || order_len:u16_be || order_utf8
event key:      0x01 0x03 || event_hash[32]
checkpoint key: 0x01 0x04 || block_height:u64_be
```

For example, main-account address `0x1111111111111111111111111111111111111111` with scope `main` encodes as prefix `0101`, twenty `11` bytes, `0004`, then UTF-8 bytes `6d61696e`. Tests assert byte-for-byte encoding and reject overlong components.

Document every key and value schema. A codec version mismatch fails startup unless a registered migration/rebuild path exists.

- [ ] **Step 3: Implement atomic write batches**

One `rocksdb::WriteBatchWithTransaction` equivalent contains all changed state keys, the `EventId -> payload_hash` idempotency entries, reconciliation rows, and the block checkpoint. Sync WAL is enabled for committed state. The batch returns only after durability policy is satisfied.

- [ ] **Step 4: Implement duplicate and collision behavior**

- Existing `EventId` with same payload hash: return `AlreadyApplied` and perform no write.
- Existing `EventId` with different payload hash: return `CriticalDivergence` and set state health red.
- Block height lower than checkpoint without explicit replay namespace: reject.

- [ ] **Step 5: Benchmark and verify**

```bash
cargo test -p hl-core --test rocks_atomicity
cargo run -p state-inspect -- verify target/test-state
cargo bench -p hl-core --bench rocks_block_batch
```

Record fsync, write, read, and compaction latency for realistic block batches in `docs/storage/rocksdb.md`.

- [ ] **Step 6: Commit**

```bash
git add services/hl-core/src/storage services/hl-core/tests/rocks_atomicity.rs config/rocksdb tools/state-inspect docs/storage/rocksdb.md Cargo.toml Cargo.lock
git commit -m "feat(state): add atomic RocksDB backend and idempotency"
```

---

### Task 3: Implement the dynamic market registry and metadata versions

**Files:**
- Create: `crates/canonical-ledger/src/market/mod.rs`
- Create: `crates/canonical-ledger/src/market/registry.rs`
- Create: `crates/canonical-ledger/src/market/metadata.rs`
- Create: `crates/canonical-ledger/src/market/reducer.rs`
- Create: `crates/canonical-ledger/tests/market_registry.rs`
- Create: `fixtures/golden/markets/market-created.json`
- Create: `fixtures/golden/markets/metadata-scale-change.json`
- Create: `fixtures/golden/markets/halt-resume.json`
- Create: `fixtures/golden/markets/dex-outcome.json`

**Interfaces:**
- Consumes: market creation/change, oracle, funding, asset context, DEX, outcome, cap, halt/resume, and margin-table events.
- Produces: point-in-time `MarketRegistry`, versioned scales/tick sizes/margin rules, market status, oracle/funding state, and metadata prerequisite checks.

- [ ] **Step 1: Write metadata-ordering and scale-change tests**

Create a block in which a metadata change precedes a trade that requires the new scale. Assert applying the trade first fails, while applying the block order succeeds. Assert historical lookup at the prior block returns the prior scale.

- [ ] **Step 2: Implement versioned metadata records**

```rust
pub enum MarketStatus { Pending, Active, Halted, Settled, Delisted }

pub struct MarketState {
    pub market_id: MarketId,
    pub metadata: MarketMetadataVersion,
    pub oracle_price: Option<Price>,
    pub funding_rate: Option<FundingRate>,
    pub open_interest: QuoteAmount,
    pub health: HealthAssessment,
}

pub struct MarketRegistry {
    pub versions: BTreeMap<MarketId, Vec<MarketMetadataVersion>>,
    pub current: BTreeMap<MarketId, MarketState>,
}

pub struct MarketMetadataVersion {
    pub market_id: MarketId,
    pub effective_from_block: BlockHeight,
    pub effective_until_block: Option<BlockHeight>,
    pub price_scale: u32,
    pub quantity_scale: u32,
    pub tick_size: Price,
    pub lot_size: Quantity,
    pub margin_table_version: String,
    pub status: MarketStatus,
    pub dex_id: Option<DexId>,
}
```

No event may parse a decimal using an unversioned global scale.

- [ ] **Step 3: Implement market reducers and prerequisites**

Reducers validate monotonic versions, non-overlapping intervals, positive tick/lot sizes, known asset links, and halt/resume transitions. Missing metadata returns `ApplyError::MissingPrerequisite` and quarantines the whole block.

- [ ] **Step 4: Add property and golden tests**

Generate valid/invalid metadata transition sequences and assert intervals never overlap and historical lookup is total over supported ranges. Run `cargo test -p canonical-ledger market_registry`; expect PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/canonical-ledger/src/market crates/canonical-ledger/tests/market_registry.rs fixtures/golden/markets
git commit -m "feat(state): add versioned dynamic market registry"
```

---

### Task 4: Implement exact account, order, position, fee, funding, and transfer reducers

**Files:**
- Create: `crates/canonical-ledger/src/account/mod.rs`
- Create: `crates/canonical-ledger/src/account/balances.rs`
- Create: `crates/canonical-ledger/src/account/orders.rs`
- Create: `crates/canonical-ledger/src/account/positions.rs`
- Create: `crates/canonical-ledger/src/account/episodes.rs`
- Create: `crates/canonical-ledger/src/account/ledger.rs`
- Create: `crates/canonical-ledger/src/account/relations.rs`
- Create: `crates/canonical-ledger/tests/account_reducers.rs`
- Create: `crates/canonical-ledger/tests/position_episodes.rs`
- Create: `fixtures/golden/accounts/standard-cross.json`
- Create: `fixtures/golden/accounts/isolated-margin.json`
- Create: `fixtures/golden/accounts/unified-account.json`
- Create: `fixtures/golden/accounts/portfolio-margin-estimated.json`
- Create: `fixtures/golden/accounts/vault-subaccount.json`

**Interfaces:**
- Consumes: all trading and account/ledger canonical events plus market metadata.
- Produces: exact protocol account state, order lifecycle, realized/unrealized accounting inputs, fees/funding/transfers, vault/subaccount relations, and analytical position episodes.

- [ ] **Step 1: Write reducer tests for every canonical event family**

Use table-driven tests that begin from an explicit state, apply one event, and assert exact state/delta/error. Include partial fills, cancel after partial fill, duplicate fill, crossing through flat, direction reversal, funding signs, builder fees, withdrawals exceeding balance, and vault/subaccount transfers.

- [ ] **Step 2: Implement exhaustive order lifecycle**

```rust
pub struct OrderKey {
    pub market_id: MarketId,
    pub order_id: OrderId,
}

pub struct OrderState {
    pub key: OrderKey,
    pub account_id: AccountId,
    pub side: Direction,
    pub accepted_quantity: Quantity,
    pub filled_quantity: Quantity,
    pub limit_price: Option<Price>,
    pub lifecycle: OrderLifecycle,
    pub created_at: ProtocolTime,
    pub updated_at: ProtocolTime,
}

pub struct PositionState {
    pub market_id: MarketId,
    pub signed_quantity: Quantity,
    pub average_entry_price: Option<Price>,
    pub realized_pnl: QuoteAmount,
    pub accrued_funding: QuoteAmount,
    pub fees: QuoteAmount,
    pub isolated_collateral: Option<UsdAmount>,
}

pub struct AccountState {
    pub account_id: AccountId,
    pub master_account_id: MasterAccountId,
    pub mode: AccountModeMetadata,
    pub collateral: BTreeMap<AssetId, Quantity>,
    pub positions: BTreeMap<MarketId, PositionState>,
    pub active_orders: BTreeMap<OrderKey, OrderState>,
    pub equity: UsdAmount,
    pub health: HealthAssessment,
}

pub enum OrderLifecycle {
    Accepted,
    Resting,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}
```

Transitions outside the protocol state machine fail the block. Filled quantity may never exceed accepted quantity; cancellation removes active book state but preserves history.

- [ ] **Step 3: Implement protocol position accounting**

Maintain signed quantity, average entry/cost basis according to protocol rules, realized PnL, accrued funding, fees, margin mode, leverage, and isolated collateral. Use exact fixed-point operations with explicit rounding methods selected from metadata/margin adapters.

- [ ] **Step 4: Implement analytical position episodes separately**

An episode starts from flat, ends at flat, and splits on direction reversal. Partial reductions remain in the same episode. Each episode records opening/closing events, peak absolute exposure, capital at risk, hold duration, realized PnL, fees, funding, and entry/exit VWAP. Episode state never feeds protocol reconciliation.

- [ ] **Step 5: Add conservation and idempotency properties**

Property tests assert buyer/seller trade quantity symmetry, explicit fee/funding sign consistency, no negative filled remainder, close-then-reopen creates a new episode, and duplicate events do not alter state.

Run:

```bash
cargo test -p canonical-ledger account position_episodes
cargo test -p canonical-ledger --test account_reducers
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/canonical-ledger/src/account crates/canonical-ledger/tests fixtures/golden/accounts
git commit -m "feat(ledger): reconstruct exact accounts orders and positions"
```

---

### Task 5: Implement margin and account-mode adapters

**Files:**
- Modify: `crates/margin-models/src/lib.rs`
- Create: `crates/margin-models/src/model.rs`
- Create: `crates/margin-models/src/standard.rs`
- Create: `crates/margin-models/src/unified.rs`
- Create: `crates/margin-models/src/portfolio.rs`
- Create: `crates/margin-models/src/isolated.rs`
- Create: `crates/margin-models/src/hip3.rs`
- Create: `crates/margin-models/src/outcome.rs`
- Create: `crates/margin-models/tests/boundaries.rs`
- Create: `fixtures/golden/margin/cross-long.json`
- Create: `fixtures/golden/margin/isolated-short.json`
- Create: `fixtures/golden/margin/portfolio-estimated.json`
- Create: `fixtures/golden/margin/liquidation-boundary.json`
- Create: `docs/models/margin-models.md`

**Interfaces:**
- Consumes: account state, market metadata, oracle state, collateral values, and versioned protocol rules.
- Produces: initial/maintenance margin, liquidation thresholds/ranges, reconciliation fields, and uncertainty flags through one versioned `MarginModel` trait.

- [ ] **Step 1: Write boundary tests from protocol examples and recorded states**

Cover exact maintenance boundary, one unit above/below, isolated collateral exhaustion, unified collateral netting, portfolio-mode uncertainty, HIP-3 fees/rules, and outcome market settlement. Expected values come from protocol fields or independently calculated reviewed fixtures.

- [ ] **Step 2: Define the model contract**

```rust
pub enum AccountModeMetadata {
    StandardCross,
    StandardIsolated { market_id: MarketId },
    Unified,
    Portfolio { rules_version: String },
    Hip3 { dex_id: DexId, rules_version: String },
    Outcome { market_id: MarketId },
}

pub struct MarginInput {
    pub account_id: AccountId,
    pub mode: AccountModeMetadata,
    pub collateral_value: UsdAmount,
    pub positions: Vec<PositionState>,
    pub oracle_prices: BTreeMap<MarketId, Price>,
    pub metadata_block: BlockHeight,
}

pub enum LiquidationEstimate {
    Exact { trigger_price: Price },
    Range { lower: Price, upper: Price, reason: String },
    NotApplicable,
}

pub enum CalculationConfidence { Exact, Bounded, Unsupported }

#[derive(Debug, thiserror::Error)]
pub enum MarginError {
    #[error("unsupported account or margin metadata version")]
    UnsupportedVersion,
    #[error("required market or oracle input is missing: {0}")]
    MissingInput(String),
    #[error("fixed-point calculation failed: {0}")]
    Calculation(String),
}

pub trait MarginModel: Send + Sync {
    fn model_id(&self) -> &'static str;
    fn supports(&self, metadata: &AccountModeMetadata) -> bool;
    fn evaluate(&self, input: &MarginInput) -> Result<MarginAssessment, MarginError>;
}

pub struct MarginAssessment {
    pub initial_margin: UsdAmount,
    pub maintenance_margin: UsdAmount,
    pub margin_ratio: MarginRatio,
    pub liquidation: LiquidationEstimate,
    pub confidence: CalculationConfidence,
    pub reconciliation_fields: BTreeMap<String, String>,
}
```

- [ ] **Step 3: Implement exact models and explicit uncertainty**

If observable data cannot reproduce a portfolio-mode threshold exactly, return a bounded `LiquidationEstimate::Range { lower, upper, reason }`. Never coerce it to an exact price. Unsupported metadata versions return `UnsupportedVersion` and health red for dependent fragility.

- [ ] **Step 4: Differentially verify against sampled official state**

A test harness imports reviewed snapshots and compares margin, leverage, and liquidation fields within exact protocol tolerance. Differences are stored as reconciliation records, not just logs.

Run `cargo test -p margin-models --all-features`; expect PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/margin-models fixtures/golden/margin docs/models/margin-models.md Cargo.toml Cargo.lock
git commit -m "feat(margin): add versioned account and liquidation models"
```

---

### Task 6: Implement L4/L2 order-book reconstruction and executable-liquidity queries

**Files:**
- Modify: `crates/orderbook/src/lib.rs`
- Create: `crates/orderbook/src/l4.rs`
- Create: `crates/orderbook/src/l2.rs`
- Create: `crates/orderbook/src/reducer.rs`
- Create: `crates/orderbook/src/invariants.rs`
- Create: `crates/orderbook/src/execution.rs`
- Create: `crates/orderbook/src/resilience.rs`
- Create: `crates/orderbook/tests/book_replay.rs`
- Create: `crates/orderbook/tests/execution_estimate.rs`
- Create: `fixtures/golden/books/snapshot-diffs.bin`
- Create: `fixtures/golden/books/gap.bin`
- Create: `fixtures/golden/books/duplicate-order.bin`
- Create: `fixtures/golden/books/crossed-book.bin`
- Create: `tools/book-inspect/src/main.rs`

**Interfaces:**
- Consumes: order status, raw diff, fill, snapshot, metadata, and latency assumptions.
- Produces: exact active L4 book, deterministic L2 views, book health, depth/imbalance/toxicity metrics, and `quote_execution` estimates.

- [ ] **Step 1: Write snapshot-plus-diff equivalence tests**

Replay a reviewed snapshot and diff sequence, then compare every active order and L2 level with an independent expected snapshot. Insert gaps, negative quantity, duplicate order IDs, and crossed invalid states; each must set book health red.

- [ ] **Step 2: Implement deterministic L4 and L2 state**

Use price-time ordering with explicit protocol order sequence. Aggregate L2 from active L4 only. Filled/cancelled orders leave active state but remain in an append-only lifecycle history outside the book.

- [ ] **Step 3: Implement book invariants and resynchronization state**

```rust
pub enum BookHealth {
    Healthy,
    AwaitingSnapshot { reason: String },
    Red { reason: String },
}
```

A gap or mismatch prevents execution estimates until a verified snapshot plus contiguous subsequent diffs restores health.

- [ ] **Step 4: Implement the execution query contract**

```rust
pub struct OrderBookState {
    pub market_id: MarketId,
    pub sequence: u64,
    pub active_orders: BTreeMap<OrderKey, OrderState>,
    pub health: BookHealth,
    pub as_of_block: BlockHeight,
}

pub struct ExecutionRequest {
    pub market_id: MarketId,
    pub side: Direction,
    pub quantity: Quantity,
    pub max_participation: ProbabilityPpm,
    pub fee_schedule_id: FeeScheduleId,
    pub exit_stress_multiplier: ProbabilityPpm,
}

pub struct ExecutionEstimate {
    pub fill_probability: ProbabilityPpm,
    pub expected_fill_quantity: Quantity,
    pub p10_vwap: Price,
    pub p50_vwap: Price,
    pub p90_vwap: Price,
    pub spread_bps: BasisPoints,
    pub impact_bps: BasisPoints,
    pub queue_uncertainty: ProbabilityPpm,
    pub time_to_fill: LatencyDistribution,
    pub normal_exit_cost_bps: BasisPoints,
    pub stressed_exit_cost_bps: BasisPoints,
    pub capacity_by_cost: BTreeMap<BasisPoints, UsdAmount>,
    pub as_of_block: BlockHeight,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("order book is not healthy")]
    BookNotHealthy,
    #[error("requested market does not match the book")]
    MarketMismatch,
    #[error("requested quantity is not positive")]
    InvalidQuantity,
    #[error("insufficient visible liquidity")]
    InsufficientLiquidity,
}

pub fn quote_execution(
    book: &OrderBookState,
    request: &ExecutionRequest,
    latency: &LatencyDistribution,
) -> Result<ExecutionEstimate, ExecutionError>;
```

`ExecutionEstimate` returns fill probability, expected fill size, P10/P50/P90 VWAP, spread, impact, queue uncertainty, time-to-fill, normal/stressed exit costs, and capacity at configured bps thresholds. No estimate is returned from a red book.

- [ ] **Step 5: Add benchmarks and differential checks**

```bash
cargo test -p orderbook
cargo run -p book-inspect -- replay fixtures/golden/books/btc-session.bin
cargo bench -p orderbook --bench book_updates
```

Record update/query latency, memory per active order, and worst-case rebuild time.

- [ ] **Step 6: Commit**

```bash
git add crates/orderbook fixtures/golden/books tools/book-inspect Cargo.toml Cargo.lock
git commit -m "feat(orderbook): reconstruct books and executable liquidity"
```

---

### Task 7: Implement the block-atomic reducer and state-delta publication boundary

**Files:**
- Create: `crates/canonical-ledger/src/reducer.rs`
- Create: `crates/canonical-ledger/src/invariants.rs`
- Create: `crates/canonical-ledger/src/partition.rs`
- Create: `crates/canonical-ledger/tests/block_atomic.rs`
- Create: `services/hl-core/src/consumer.rs`
- Create: `services/hl-core/src/apply.rs`
- Create: `services/hl-core/src/publish.rs`
- Create: `services/hl-core/tests/crash_matrix.rs`

**Interfaces:**
- Consumes: committed canonical block stream, market/margin/order-book/account reducers, RocksDB backend.
- Produces: atomic `apply_block`, canonical state hash, ordered `StateDelta`, block checkpoint, and post-commit state publications.

- [ ] **Step 1: Write block rollback and ordering tests**

Create a block whose final event violates an invariant. Assert no prior event from the block becomes visible, checkpoint remains unchanged, and the same pre-block state hash is retained. Create a valid block with metadata, book, trade, and ledger events and assert the design-specified application order.

- [ ] **Step 2: Implement the pure block reducer**

```rust
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("block is not the next contiguous committed block")]
    NonContiguousBlock,
    #[error("missing prerequisite: {0}")]
    MissingPrerequisite(String),
    #[error("event {event_id} is unsupported by schema {schema_version}")]
    UnsupportedEvent { event_id: EventId, schema_version: String },
    #[error("event identity collision for {0}")]
    EventIdentityCollision(EventId),
    #[error("state invariant failed: {0}")]
    InvariantViolation(String),
    #[error("fixed-point calculation failed: {0}")]
    Calculation(String),
}

pub struct EventReceipt {
    pub event_id: EventId,
    pub payload_hash: [u8; 32],
    pub change_count: u32,
    pub post_event_scope_hash: [u8; 32],
}

pub struct InvariantReport {
    pub checks_run: u32,
    pub warnings: Vec<String>,
    pub passed: bool,
    pub report_hash: [u8; 32],
}

pub fn reduce_block(
    state: &CanonicalState,
    block: &BlockEnvelope,
) -> Result<PreparedBlock, ApplyError>;

pub struct PreparedBlock {
    pub next_state_changes: StateDelta,
    pub event_receipts: Vec<EventReceipt>,
    pub state_hash: [u8; 32],
    pub invariant_report: InvariantReport,
}
```

The function performs no I/O, wall-clock reads, randomness, or global mutation.

- [ ] **Step 3: Implement deterministic partition preparation**

Pure account preparation may run in parallel by stable hash partition. Results are merged by partition ID, account ID, and event order. Cross-account trade effects originate from one `TradeMatched` reducer and are applied together.

- [ ] **Step 4: Implement consume-prepare-commit-publish ordering**

`hl-core` pulls one block, prepares it, writes one RocksDB batch, then publishes state deltas. It acknowledges the canonical message only after state commit and successful durable state-delta publication or a persisted republish marker. Crash tests verify safe recovery at every boundary.

- [ ] **Step 5: Run atomicity, property, and Loom tests**

```bash
cargo test -p canonical-ledger block_atomic
cargo test -p hl-core --test crash_matrix
cargo test -p hl-core --features loom
```

Expected: PASS with identical state/delta hashes across repeated runs.

- [ ] **Step 6: Commit**

```bash
git add crates/canonical-ledger/src/reducer.rs crates/canonical-ledger/src/invariants.rs crates/canonical-ledger/src/partition.rs crates/canonical-ledger/tests/block_atomic.rs services/hl-core/src/consumer.rs services/hl-core/src/apply.rs services/hl-core/src/publish.rs services/hl-core/tests/crash_matrix.rs
git commit -m "feat(core): apply committed blocks atomically"
```

---

### Task 8: Implement checkpoints, state snapshots, and continuous reconciliation

**Files:**
- Create: `services/hl-core/src/checkpoint.rs`
- Create: `services/hl-core/src/reconciliation/mod.rs`
- Create: `services/hl-core/src/reconciliation/accounts.rs`
- Create: `services/hl-core/src/reconciliation/books.rs`
- Create: `services/hl-core/src/reconciliation/sources.rs`
- Create: `services/hl-core/tests/checkpoint_restore.rs`
- Create: `schemas/clickhouse/0001_reconciliation.sql`
- Create: `tools/state-diff/src/main.rs`
- Create: `docs/runbooks/state-mismatch.md`
- Create: `docs/runbooks/checkpoint-restore.md`

**Interfaces:**
- Consumes: canonical archive manifests, RocksDB state, independent snapshots/API samples, book snapshots, build/schema provenance.
- Produces: filesystem-consistent checkpoints, checkpoint manifests, reconciliation results, scoped health, and reproducible state-diff bundles.

The checkpoint contract is:

```rust
pub struct BlockCheckpoint {
    pub chain_id: ChainId,
    pub block_height: BlockHeight,
    pub canonical_block_hash: [u8; 32],
    pub canonical_state_hash: [u8; 32],
    pub archive_manifest_id: ManifestId,
    pub codec_version: u32,
    pub build_commit: String,
    pub created_at: KnownTime,
}
```

A checkpoint is valid only when the referenced archive manifest verifies and replay to `block_height` reproduces `canonical_state_hash`.

- [ ] **Step 1: Write checkpoint restore equivalence tests**

Build state through block 10,000, create a checkpoint, restore into a clean directory, verify the checkpoint state hash, apply blocks 10,001–10,500, and compare with a full replay. Hashes and reconciliation results must match.

- [ ] **Step 2: Implement checkpoint manifests**

Each manifest contains block height, state hash, RocksDB manifest/file hashes, canonical archive manifest hash, canonical schema version, state codec version, market metadata version set, build ID, and creation time. Copy/replication is considered complete only after manifest verification.

- [ ] **Step 3: Implement stored reconciliation outcomes**

Record contiguous heights, source hashes, buyer/seller symmetry, order-size invariants, conservation checks, fee/funding signs, book snapshot comparison, sampled account differences, and checkpoint hash checks in ClickHouse and RocksDB health state.

- [ ] **Step 4: Implement `state-diff` evidence bundles**

`state-diff` compares two checkpoints or one checkpoint and external snapshot, outputs sorted machine-readable differences, source event references, market/account scope, tolerances, and a replay command. No unordered map output is permitted.

- [ ] **Step 5: Test restoration and mismatch suppression**

```bash
cargo test -p hl-core --test checkpoint_restore
cargo run -p state-diff -- checkpoint target/a target/b --output target/diff.json
```

Expected: equivalent checkpoints produce an empty difference; injected mismatch creates red scoped health and suppresses dependent publications.

- [ ] **Step 6: Commit**

```bash
git add services/hl-core/src/checkpoint.rs services/hl-core/src/reconciliation services/hl-core/tests/checkpoint_restore.rs schemas/clickhouse/0001_reconciliation.sql tools/state-diff docs/runbooks/state-mismatch.md docs/runbooks/checkpoint-restore.md
git commit -m "feat(state): add checkpoints and persistent reconciliation"
```

---

### Task 9: Integrate and benchmark the `hl-core` service

**Files:**
- Modify: `services/hl-core/src/main.rs`
- Create: `services/hl-core/src/app.rs`
- Create: `services/hl-core/src/config.rs`
- Create: `services/hl-core/src/status.rs`
- Create: `services/hl-core/tests/end_to_end.rs`
- Create: `config/core.example.toml`
- Create: `infra/systemd/hl-core.service.d/override.conf`
- Create: `infra/monitoring/dashboards/core-state.json`
- Create: `infra/monitoring/alerts/core-state.yml`
- Create: `docs/runbooks/core-restart.md`

**Interfaces:**
- Consumes: canonical JetStream subjects and immutable archive fallback.
- Produces: hardened `hl-core`, exact hot state, state deltas, book deltas, checkpoints, reconciliation metrics, and health endpoints.

- [ ] **Step 1: Write end-to-end archive-to-state tests**

Replay the Stage 1 corpus through `hl-core`, restart twice, and assert known state hash, account snapshots, active orders, positions, episode counts, book hash, and checkpoint height.

- [ ] **Step 2: Implement startup and recovery order**

Startup verifies config/schema/build, opens/validates RocksDB column families, restores or validates checkpoint, catches up from archive if JetStream retention expired, creates durable consumers, and exposes readiness only after committed state and required book scopes are healthy.

- [ ] **Step 3: Add workload isolation and backpressure**

Separate bounded tasks for canonical consumption, state commit, state publication, reconciliation, and checkpointing. ClickHouse/analytics outages must not block state commit. Research workloads cannot access the hot RocksDB path directly.

- [ ] **Step 4: Measure SLOs and resource profiles**

Replay at 5x observed 30-day P99 and 10x average. Record p50/p95/p99 block apply, RocksDB commit, book update, state publication, CPU, memory, and compaction impact. The target is p99 committed block to canonical state below 150 ms on target hardware.

- [ ] **Step 5: Verify degraded modes**

Disconnect NATS after archive catch-up, delay reconciliation, force ClickHouse unavailable, and inject a book mismatch. Live state must continue where policy permits; dependent estimates/signals must be suppressed exactly as designed.

- [ ] **Step 6: Commit**

```bash
git add services/hl-core config/core.example.toml infra/systemd/hl-core.service.d infra/monitoring/dashboards/core-state.json infra/monitoring/alerts/core-state.yml docs/runbooks/core-restart.md
git commit -m "feat(core): integrate deterministic state service"
```

---

### Task 10: Execute the Stage 2 state-reconstruction gate

**Files:**
- Create or modify before verification: `config/stage-gates/stage-2.toml`
- Create before verification: `tests/regression/state/manifest.toml`
- Create or modify before verification: `justfile`
- Generate after verification: `docs/stage-gates/stage-2-state-reconstruction.evidence.json`
- Generate after verification: `docs/stage-gates/stage-2-state-reconstruction.md`

**Interfaces:**
- Consumes: the complete stage implementation, approved point-in-time regression material, and prior signed gate evidence.
- Produces: a clean-commit canonical gate report, signed approval record, and signed `stage-2-state` tag.

- [ ] **Step 1: Freeze the regression and review inputs**

Freeze the canonical archive hash, final state hash, checkpoint hashes, sampled account states, position episodes, order-lifecycle counts, L4/L2 book hashes, margin assessments, reconciliation tolerances, and external-snapshot evidence for the approved corpus.

- [ ] **Step 2: Implement the exact gate configuration and tests**

`just stage-2-gate` writes only to ignored `target/stage-gates/stage-2.json` and runs all unit, property, golden, differential, fuzz-smoke, and concurrency tests; full replay from archive; replay from every retained checkpoint version; duplicate-delivery and crash matrices; sampled account reconciliation; independent book snapshot comparison; and 5x P99 plus 10x average load tests.

The gate runner must reject a dirty worktree before any check, record the clean implementation SHA, and fail closed on missing evidence or approvals. Add a configuration test proving every required command and artifact is present.

- [ ] **Step 3: Commit every gate input before verification**

```bash
git add config/stage-gates/stage-2.toml tests/regression/state justfile
git commit -m "chore(gate): add Stage 2 state reconstruction verification inputs"
test -z "$(git status --porcelain)"
git rev-parse HEAD
```

The printed SHA is the immutable implementation commit evaluated by this gate.

- [ ] **Step 4: Run the gate from fresh clean clones on two supported hosts**

```bash
just stage-2-gate
cargo run -p state-diff -- regression tests/regression/state/manifest.toml --output target/stage-gates/stage-2-state-diff.json
sha256sum target/stage-gates/stage-2.json
```

Expected: PASS; canonical report, state/output hashes, and configured reproducibility views agree across hosts. Host-specific provenance remains recorded but is excluded only from the explicitly defined cross-host comparison projection.

- [ ] **Step 5: Commit evidence, collect approvals, and sign the stage tag**

```bash
cp target/stage-gates/stage-2.json docs/stage-gates/stage-2-state-reconstruction.evidence.json
cargo run -p stage-gate -- render-record --evidence docs/stage-gates/stage-2-state-reconstruction.evidence.json --output docs/stage-gates/stage-2-state-reconstruction.md
git add docs/stage-gates/stage-2-state-reconstruction.evidence.json docs/stage-gates/stage-2-state-reconstruction.md
git commit -m "docs(gate): record Stage 2 state reconstruction evidence"
git tag -s stage-2-state -m "Stage 2 state reconstruction gate passed"
git verify-tag stage-2-state
```

Platform/data and independent reviewers must provide the detached approval artifacts referenced by the record. Do not create the tag when a required check, comparison, review, or bounded-limitation statement is missing.

## Stage 2 Exit Criteria

- Repeated full replay and checkpoint replay produce identical deterministic state hashes.
- Account, order, position, fee, funding, transfer, market, margin, and book state reconcile within exact approved tolerances.
- Duplicate delivery and process crashes do not duplicate or partially apply effects.
- Unsupported account modes and book mismatches are explicit and suppress dependent intelligence.
- A clean host rebuilds state from archive plus compatible checkpoint.
- `docs/stage-gates/stage-2-state-reconstruction.md` is approved and tag `stage-2-state` exists.
