# Canonical Account Ledger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace every opaque V1 account/risk payload with strict typed
contracts and reconstruct deterministic account cash-flow, relation, mode,
ordinary order-fill position, fee, funding, liquidation, settlement, and
analytical-episode state without inventing balances or margin rules that the
current source contract does not provide.

**Architecture:** `canonical-events` validates all nineteen V1 account and risk
payloads while retaining the enclosing wire bytes. `canonical-ledger` adds a
bounded account reducer and a fixed composite reducer that can apply market,
order, trade, and account mutations atomically. Immutable archive/checkpoint
replay proves generated canonical behavior and reports unresolved source,
balance, TWAP, backstop-basis, and margin boundaries explicitly.

**Tech Stack:** Rust 1.97.1, Prost canonical V1, checked `i128` fixed-point
domain types, BLAKE3 state hashes, strict canonical JSON records, immutable
Parquet archive, private local checkpoints.

## Global Constraints

- V1 remains read-only: no signer, private key, order placement, custody, or
  execution route may be added.
- Support exact canonical schema `1.0.0` only; unknown versions, modes, fee
  types, and transitions fail closed.
- Preserve enclosing canonical envelope bytes containing valid unknown
  Protobuf fields.
- `f32` and `f64` are forbidden in balances, positions, fees, funding, PnL,
  margin, liquidation, identity, state hashing, and reconciliation.
- Values with different decimal scales are normalized only by exact
  scale-increasing operations; no rounding may enter canonical state.
- Market-dependent state requires an exact current market metadata version;
  unresolved metadata suppresses the dependent transition.
- A block commits all market/order/trade/account mutations or none.
- Current V1 does not prove opening venue balances, deposit clearinghouse
  routing, TWAP side, backstop transfer price/cost basis, or portfolio-margin
  rules. These remain explicit unresolved/false qualifications.
- Source qualification remains `synthetic_unassessed` until immutable
  committed-node/operator schemas and recordings are separately mapped and
  qualified.
- Every task follows red-green-refactor, receives a fresh review gate, and
  commits only its owned files.

---

### Task 1: Type cash-flow, transfer, and vault payloads

**Files:**
- Modify: `crates/api-contracts/src/lib.rs`
- Modify: `crates/api-contracts/tests/payload_codec.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Create: `crates/canonical-events/tests/account_payloads.rs`

**Interfaces:**
- Consumes: V1 `DepositCredited`, `WithdrawalDebited`, `SpotTransfer`,
  `PerpTransfer`, `SubaccountTransfer`, `VaultDeposit`, and
  `VaultWithdrawal` Protobuf messages.
- Produces: public payload structs using `Address`, `AssetId`, `VaultId`,
  `Quantity`, and `QuoteAmount`, with deterministic API encode/decode helpers.

- [x] **Step 1: Write the failing payload tests**

  Add exact round-trip tests for all seven kinds. Assert lowercase API
  addresses, distinct transfer endpoints, positive amounts/shares, bounded
  reference strings (`1..=256` bytes, no ASCII controls), matching event kind,
  original enclosing wire preservation, and rejection of blank/padded IDs,
  zero/negative values, malformed decimals, and over-precision above the
  domain maximum.

- [x] **Step 2: Run the red test**

  ```bash
  cargo +1.97.1 test -p canonical-events --test account_payloads --locked --offline
  ```

  Expected: compile failure because the seven typed payload APIs do not exist.

- [x] **Step 3: Implement the strict payloads**

  Define:

  ```rust
  pub struct DepositCredited {
      pub account_id: Address,
      pub asset_id: AssetId,
      pub amount: Quantity,
      pub deposit_reference: String,
  }

  pub struct WithdrawalDebited {
      pub account_id: Address,
      pub asset_id: AssetId,
      pub amount: Quantity,
      pub withdrawal_reference: String,
  }

  pub struct SpotTransfer {
      pub from_account_id: Address,
      pub to_account_id: Address,
      pub asset_id: AssetId,
      pub amount: Quantity,
  }

  pub struct PerpTransfer {
      pub from_account_id: Address,
      pub to_account_id: Address,
      pub quote_amount: QuoteAmount,
  }

  pub struct SubaccountTransfer {
      pub master_account_id: Address,
      pub from_account_id: Address,
      pub to_account_id: Address,
      pub asset_id: AssetId,
      pub amount: Quantity,
  }

  pub struct VaultDeposit {
      pub vault_id: VaultId,
      pub account_id: Address,
      pub amount: QuoteAmount,
      pub shares_issued: Quantity,
  }

  pub struct VaultWithdrawal {
      pub vault_id: VaultId,
      pub account_id: Address,
      pub amount: QuoteAmount,
      pub shares_redeemed: Quantity,
  }
  ```

  Remove only these seven variants from `opaque_payloads!`. Preserve the
  enclosing envelope byte path; canonical payload re-encoding must be strict
  and deterministic.

- [x] **Step 4: Verify and commit**

  ```bash
  cargo +1.97.1 test -p api-contracts -p canonical-events --locked --offline
  cargo +1.97.1 clippy -p api-contracts -p canonical-events --all-targets --all-features --locked --offline -- -D warnings
  git diff --check
  git add crates/api-contracts crates/canonical-events
  git commit -m "feat(events): type canonical account cash flows"
  ```

---

### Task 2: Type fees, funding, rewards, and account modes

**Files:**
- Modify: `crates/domain-types/src/ids.rs`
- Modify: `crates/domain-types/src/shared.rs`
- Modify: `crates/domain-types/src/lib.rs`
- Modify: `crates/domain-types/tests/ids.rs`
- Modify: `crates/domain-types/tests/shared.rs`
- Modify: `crates/api-contracts/src/lib.rs`
- Modify: `crates/api-contracts/tests/payload_codec.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Modify: `crates/canonical-events/tests/account_payloads.rs`

**Interfaces:**
- Consumes: `FeeCharged`, `BuilderFeeCharged`, `FundingPaid`,
  `FundingReceived`, `ReferralReward`, `AccountModeChanged`,
  `MarginModeChanged`, and `LeverageChanged`.
- Produces: `FeeTypeV1`, `AccountAbstractionModeV1`, `MarginModeV1`, and strict
  typed payloads. `FundingRate` remains signed; paid/received amounts are
  positive and their event kind supplies direction. `FeeRate` remains signed:
  `maker_rebate` requires a negative rate and credits the account, while every
  charged fee type requires a positive rate and debits the account.

- [x] **Step 1: Write the failing domain and payload tests**

  Accept only these frozen wire values:

  ```text
  fee_type: maker | taker | maker_rebate | referral_discount | protocol
  account_mode: standard | unified | portfolio | dex_abstraction
  margin_mode: cross | isolated | strict_isolated
  ```

  Reject unknown/case-folded/padded values, same previous/new modes, same
  previous/new leverage, non-positive amounts or leverage, a nonnegative
  `maker_rebate` rate, a nonpositive charged fee rate, duplicate
  builder/referrer endpoints, malformed addresses/IDs, and noncanonical direct
  payload bytes. Cross-checking payload identities against enclosing
  account/market identities belongs to the block-atomic reducer in Task 4,
  not the payload codec.

- [x] **Step 2: Run red**

  ```bash
  cargo +1.97.1 test -p domain-types -p canonical-events --test account_payloads --locked --offline
  ```

- [x] **Step 3: Implement exact types and codecs**

  Define payloads with these fields:

  ```rust
  pub struct FeeCharged {
      pub account_id: Address,
      pub asset_id: AssetId,
      pub amount: Quantity,
      pub fee_rate: FeeRate,
      pub fee_type: FeeTypeV1,
  }

  pub struct BuilderFeeCharged {
      pub account_id: Address,
      pub builder_account_id: Address,
      pub asset_id: AssetId,
      pub amount: Quantity,
  }

  pub struct FundingPaid {
      pub account_id: Address,
      pub market_id: MarketId,
      pub amount: QuoteAmount,
      pub funding_rate: FundingRate,
  }

  pub struct FundingReceived {
      pub account_id: Address,
      pub market_id: MarketId,
      pub amount: QuoteAmount,
      pub funding_rate: FundingRate,
  }

  pub struct ReferralReward {
      pub account_id: Address,
      pub referrer_account_id: Address,
      pub asset_id: AssetId,
      pub amount: Quantity,
  }

  pub struct AccountModeChanged {
      pub account_id: Address,
      pub previous_mode: AccountAbstractionModeV1,
      pub new_mode: AccountAbstractionModeV1,
  }

  pub struct MarginModeChanged {
      pub account_id: Address,
      pub market_id: MarketId,
      pub previous_mode: MarginModeV1,
      pub new_mode: MarginModeV1,
  }

  pub struct LeverageChanged {
      pub account_id: Address,
      pub market_id: MarketId,
      pub previous_leverage: Leverage,
      pub new_leverage: Leverage,
  }
  ```

  Remove only these eight variants from `opaque_payloads!`. Keep builder and
  referrer counterpart identities distinct from the primary account.

- [x] **Step 4: Verify and commit**

  ```bash
  cargo +1.97.1 test -p domain-types -p api-contracts -p canonical-events --locked --offline
  cargo +1.97.1 clippy -p domain-types -p api-contracts -p canonical-events --all-targets --all-features --locked --offline -- -D warnings
  git diff --check
  git add crates/domain-types crates/api-contracts crates/canonical-events
  git commit -m "feat(events): type canonical fees funding and account modes"
  ```

---

### Task 3: Type liquidation and settlement payloads

**Files:**
- Modify: `crates/domain-types/src/ids.rs`
- Modify: `crates/domain-types/src/lib.rs`
- Modify: `crates/domain-types/tests/ids.rs`
- Modify: `crates/api-contracts/src/lib.rs`
- Modify: `crates/api-contracts/tests/payload_codec.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Modify: `crates/canonical-events/tests/account_payloads.rs`

**Interfaces:**
- Consumes: `LiquidationStarted`, `LiquidationFill`,
  `BackstopLiquidation`, and `PositionSettled`.
- Produces: `LiquidationId` and four typed risk payloads. A backstop event does
  not contain transfer price or cost basis; the later reducer must mark
  affected position accounting unresolved instead of inventing either value.

- [x] **Step 1: Write failing risk tests**

  Cover valid exact round trips and reject malformed addresses/IDs,
  zero/negative fill price or quantity, negative margin values, equal
  liquidated/backstop accounts, and a settled quantity of zero.
  `PositionSettled.realized_pnl` remains signed. Settlement price is
  nonnegative so an outcome may settle at zero. Cross-checking payload
  identities against enclosing account/market identities remains a single
  block-atomic reducer responsibility in Task 4.

- [x] **Step 2: Run red**

  ```bash
  cargo +1.97.1 test -p canonical-events --test account_payloads --locked --offline
  ```

- [x] **Step 3: Implement the risk payloads**

  ```rust
  pub struct LiquidationStarted {
      pub account_id: Address,
      pub liquidation_id: LiquidationId,
      pub margin_value: UsdAmount,
      pub maintenance_requirement: UsdAmount,
  }

  pub struct LiquidationFill {
      pub liquidation_id: LiquidationId,
      pub account_id: Address,
      pub market_id: MarketId,
      pub price: Price,
      pub quantity: Quantity,
  }

  pub struct BackstopLiquidation {
      pub liquidation_id: LiquidationId,
      pub account_id: Address,
      pub backstop_account_id: Address,
      pub market_id: MarketId,
      pub quantity: Quantity,
  }

  pub struct PositionSettled {
      pub account_id: Address,
      pub market_id: MarketId,
      pub settlement_price: Price,
      pub settled_quantity: Quantity,
      pub realized_pnl: QuoteAmount,
  }
  ```

  Require `margin_value` and `maintenance_requirement` to share a scale and
  require `margin_value < maintenance_requirement` for
  `LiquidationStarted`.

- [x] **Step 4: Verify and commit**

  ```bash
  cargo +1.97.1 test -p domain-types -p api-contracts -p canonical-events --locked --offline
  cargo +1.97.1 clippy -p domain-types -p api-contracts -p canonical-events --all-targets --all-features --locked --offline -- -D warnings
  git diff --check
  git add crates/domain-types crates/api-contracts crates/canonical-events
  git commit -m "feat(events): type canonical liquidation payloads"
  ```

---

### Task 4: Reconstruct account facts, cash-flow totals, relations, and modes

**Files:**
- Create: `crates/canonical-ledger/src/account/mod.rs`
- Create: `crates/canonical-ledger/src/account/codec.rs`
- Create: `crates/canonical-ledger/src/account/cashflow.rs`
- Create: `crates/canonical-ledger/src/account/relations.rs`
- Create: `crates/canonical-ledger/src/account/modes.rs`
- Modify: `crates/canonical-ledger/src/lib.rs`
- Create: `crates/canonical-ledger/tests/account_cashflow.rs`
- Modify: `docs/contracts/deterministic-state-v1.md`

**Interfaces:**
- Consumes: the fifteen typed cash-flow/fee/funding/reward/mode payloads.
- Produces: `CanonicalAccountReducerV1`, immutable facts, per-account
  cash-flow totals, vault/subaccount relations, and current account,
  margin-mode, and leverage settings.

**Execution split:** implement and independently review this task in two serial
slices. Slice A freezes bounded key-bound records and codecs without event
reduction. Slice B adds the reducer, prerequisite checks, transitions, and
block-atomic tests. The final review covers the complete Task 4 range. This
keeps unit/storage contracts reviewable before state-machine behavior depends
on them.

- [ ] **Step 1: Write failing reducer and codec tests**

  Assert one immutable fact per accepted event; exact debit/credit symmetry;
  fee/funding direction; builder/referrer counterpart totals; vault principal
  and share conservation; non-overlapping subaccount ownership; previous mode
  and leverage binding; exact payload/envelope account and market identities;
  market/asset prerequisite checks; identity collision; strict
  unknown-field-denying key-bound codecs; 16 KiB record bounds; 64 KiB
  pre-allocation key bounds; unsupported schema denial; and whole-block
  rollback after a late invalid transfer.

- [ ] **Step 2: Run red**

  ```bash
  cargo +1.97.1 test -p canonical-ledger --test account_cashflow --locked --offline
  ```

- [ ] **Step 3: Implement bounded records**

  Freeze reducer version
  `hyperliquid-alpha-desk-canonical-account@1.0.0`. Use two flow record
  families so asset/share `Quantity` values can never be interchanged with
  quote-denominated `QuoteAmount` values:

  ```rust
  pub enum AccountQuantityFlowScopeV1 {
      ExternalAsset { asset_id: AssetId },
      SpotTransferAsset { asset_id: AssetId },
      SubaccountTransferAsset { asset_id: AssetId },
      FeeAsset { asset_id: AssetId },
      BuilderFeeAsset { asset_id: AssetId },
      ReferralRewardAsset { asset_id: AssetId },
      VaultShares { vault_id: VaultId },
  }

  pub struct AccountQuantityFlowCurrentRecordV1 {
      pub account_id: Address,
      pub scope: AccountQuantityFlowScopeV1,
      pub credits: Quantity,
      pub debits: Quantity,
      pub last_event_id: EventId,
      pub last_block_height: BlockHeight,
  }

  pub enum AccountQuoteFlowScopeV1 {
      DefaultPerpQuote,
      MarketFunding { market_id: MarketId },
      VaultPrincipal { vault_id: VaultId },
  }

  pub struct AccountQuoteFlowCurrentRecordV1 {
      pub account_id: Address,
      pub scope: AccountQuoteFlowScopeV1,
      pub credits: QuoteAmount,
      pub debits: QuoteAmount,
      pub last_event_id: EventId,
      pub last_block_height: BlockHeight,
  }

  pub struct VaultPrincipalFlowCurrentRecordV1 {
      pub vault_id: VaultId,
      pub deposits: QuoteAmount,
      pub withdrawals: QuoteAmount,
      pub last_event_id: EventId,
      pub last_block_height: BlockHeight,
  }

  pub struct VaultShareFlowCurrentRecordV1 {
      pub vault_id: VaultId,
      pub shares_issued: Quantity,
      pub shares_redeemed: Quantity,
      pub last_event_id: EventId,
      pub last_block_height: BlockHeight,
  }

  pub struct AccountModeCurrentRecordV1 {
      pub account_id: Address,
      pub initial_previous: AccountAbstractionModeV1,
      pub current: AccountAbstractionModeV1,
      pub first_event_id: EventId,
      pub last_event_id: EventId,
      pub last_block_height: BlockHeight,
  }
  ```

  Add analogous key-bound margin-mode and leverage records retaining their
  initial asserted predecessor, plus `AccountFactRecordV1`,
  `SubaccountMasterCurrentRecordV1`, and
  `AccountVaultRelationCurrentRecordV1`. Relation records mean only “observed
  by an accepted canonical event”; they do not claim active ownership,
  current balance, or a complete venue hierarchy.

  Every record is strict field-ordered JSON with unknown fields denied,
  canonical byte re-encoding, a 16 KiB value ceiling, and typed `decode_at`
  key binding. Keys frame account, scope discriminator, and optional
  asset/market/vault identity; builders compute the complete size against the
  64 KiB absolute ceiling and use fallible exact reservation before copying.
  Normalize additions only upward to the greatest observed scale, which is
  exact and requires no rounding. Name all totals `flow`, never `balance`: V1
  lacks an opening snapshot and cannot prove venue balance.

- [ ] **Step 4: Implement transitions and invariants**

  Enforce exact ordered payload/envelope identity lists for every owned event.
  Deposits/withdrawals affect external-asset flow only. Spot and perp
  transfers create equal debit/credit facts in their exact scopes.
  Subaccount transfers use their own asset scope and create exactly one direct
  master relation only when `master_account_id` equals exactly one endpoint;
  the other endpoint is the subaccount. Three-distinct-account payloads fail
  closed as relationship-ambiguous, and a subaccount cannot acquire a second
  master. Do not infer transitive hierarchy or acyclicity.

  Vault deposits atomically debit quote principal and credit shares for the
  account while increasing the vault's observed deposited-principal and
  issued-share totals; withdrawals credit account quote principal and debit
  account shares while increasing the vault's observed withdrawn-principal
  and redeemed-share totals. Require account-side and vault-side totals to
  reconcile independently within each unit. Principal and shares are never
  compared or presented as current holdings.

  Builder fees debit the charged account and credit the builder in the same
  asset scope. Referral rewards credit `referrer_account_id` only; the payload
  does not identify a debit source, so never debit `account_id` or invent a
  protocol counterparty. This freezes the documented builder/referrer
  recipient meaning while leaving deployed source mapping separately
  unqualified.

  Funding direction is fixed by event kind.
  `FeeCharged` direction is fixed by `FeeTypeV1`: `maker_rebate` is a credit
  and every other frozen fee type is a debit. Rewards credit the explicit
  referrer/reward recipient recorded by the typed payload. First
  account-mode, margin-mode, and leverage transitions retain their asserted
  predecessor; later transitions must match current state exactly.

  Every payload carrying `asset_id` requires a valid current asset-context
  record. Funding, margin-mode, and leverage events require a current market
  record whose metadata resolution is exact. Perp transfers and vault events
  carry no authoritative asset/market identity, so do not synthesize a
  prerequisite. Tests seed market/asset prerequisites through a test-only
  fixed dispatcher; the production composite remains Task 5.

- [ ] **Step 5: Verify and commit**

  ```bash
  cargo +1.97.1 test -p canonical-ledger --test account_cashflow --locked --offline
  cargo +1.97.1 test -p canonical-ledger -p replay-engine --locked --offline
  cargo +1.97.1 clippy -p canonical-ledger -p replay-engine --all-targets --all-features --locked --offline -- -D warnings
  git diff --check
  git add crates/canonical-ledger docs/contracts/deterministic-state-v1.md
  git commit -m "feat(ledger): reconstruct canonical account cash flows"
  ```

---

### Task 5: Compose reducers and reconstruct ordinary positions and episodes

**Files:**
- Create: `crates/canonical-ledger/src/composite.rs`
- Create: `crates/canonical-ledger/src/account/positions.rs`
- Create: `crates/canonical-ledger/src/account/episodes.rs`
- Create: `crates/canonical-ledger/src/account/liquidations.rs`
- Modify: `crates/canonical-ledger/src/account/mod.rs`
- Modify: `crates/canonical-ledger/src/lib.rs`
- Create: `crates/canonical-ledger/tests/composite_account_state.rs`
- Create: `crates/canonical-ledger/tests/position_episodes.rs`
- Modify: `docs/contracts/deterministic-state-v1.md`

**Interfaces:**
- Consumes: `CanonicalMarketReducerV1`, `CanonicalOrderReducerV1`,
  `CanonicalTradeReducerV1`, `CanonicalAccountReducerV1`, ordinary accepted
  order fills, fees/funding, and typed liquidation/settlement events.
- Produces: `CanonicalStateReducerV1`, protocol position records, analytical
  episodes, and liquidation state under reducer version
  `hyperliquid-alpha-desk-canonical-state@1.0.0`.

- [ ] **Step 1: Write failing composite and accounting tests**

  Prove a single block can create market/order/account effects atomically.
  Cover long open/add/reduce/flat, short open/add/reduce/flat, direction
  reversal, exact entry VWAP, realized PnL, fee/funding attribution, episode
  start/close/split, liquidation start/fill, settlement, duplicate fill,
  fill over remaining quantity, wrong account/market, unresolved metadata,
  backstop unresolved-basis state, and a late error rolling back every
  component mutation.

- [ ] **Step 2: Run red**

  ```bash
  cargo +1.97.1 test -p canonical-ledger --test composite_account_state --test position_episodes --locked --offline
  ```

- [ ] **Step 3: Implement fixed composite dispatch**

  `CanonicalStateReducerV1` invokes every component that owns an event and
  concatenates disjoint mutations in fixed order:

  ```text
  market -> order -> trade -> account
  ```

  Every component sees the same pre-event candidate state; subsequent events
  in the block see all prior event mutations. Component state-key collisions
  fail the block. `validate_block` invokes all component invariants.

- [ ] **Step 4: Implement exact ordinary-fill accounting**

  `OrderAccepted` establishes account/market/side ownership.
  `OrderPartiallyFilled` and `OrderFilled` read the prior key-bound order
  record. Normalize price and quantity upward to exact current market scales,
  require tick/lot alignment, and use checked fixed-point arithmetic.

  ```rust
  pub enum PositionResolutionV1 {
      Exact,
      UnresolvedBackstop { liquidation_id: LiquidationId },
      UnresolvedTwapDirection,
  }

  pub struct PositionCurrentRecordV1 {
      pub account_id: Address,
      pub market_id: MarketId,
      pub signed_quantity: Quantity,
      pub average_entry_price: Option<Price>,
      pub realized_pnl: QuoteAmount,
      pub accrued_funding: QuoteAmount,
      pub fees: QuoteAmount,
      pub resolution: PositionResolutionV1,
      pub last_event_id: EventId,
      pub last_block_height: BlockHeight,
  }
  ```

  Same-direction fills update weighted entry. Opposite-direction fills realize
  PnL on the closed quantity; a reversal closes the old episode and opens a
  new episode at the fill price. Flat positions have no entry price.

- [ ] **Step 5: Implement risk behavior without invented cost basis**

  `LiquidationFill` reduces the existing side at its explicit price.
  `PositionSettled` applies its explicit realized PnL and settled quantity.
  `BackstopLiquidation` records immutable facts and changes affected position
  resolution to `UnresolvedBackstop`; it must not manufacture a transfer
  price, entry basis, or recipient PnL. Any later exact-PnL-dependent
  transition fails until an authoritative snapshot/resolution contract exists.
  TWAP payloads remain outside exact position state because V1 omits side.

- [ ] **Step 6: Verify and commit**

  ```bash
  cargo +1.97.1 test -p canonical-ledger --test composite_account_state --test position_episodes --locked --offline
  cargo +1.97.1 test -p canonical-ledger -p replay-engine --locked --offline
  cargo +1.97.1 clippy -p canonical-ledger -p replay-engine --all-targets --all-features --locked --offline -- -D warnings
  git diff --check
  git add crates/canonical-ledger docs/contracts/deterministic-state-v1.md
  git commit -m "feat(state): reconstruct canonical positions and episodes"
  ```

---

### Task 6: Add retained composite account replay evidence

**Files:**
- Create: `tools/state-replay/src/account.rs`
- Modify: `tools/state-replay/src/lib.rs`
- Modify: `tools/state-replay/src/main.rs`
- Create: `tools/state-replay/tests/account_e2e.rs`
- Modify: `tools/state-replay/tests/cli.rs`
- Modify: `justfile`
- Modify: `README.md`
- Modify: `docs/STATUS.md`
- Modify: `docs/contracts/deterministic-state-v1.md`
- Modify: `docs/runbooks/state-replay-evidence.md`
- Modify: `docs/superpowers/plans/2026-07-29-canonical-account-ledger.md`

**Interfaces:**
- Consumes: `CanonicalStateReducerV1`, immutable archive, checkpoint store,
  serial replay, and generated canonical events only.
- Produces: `state-replay account-e2e`, bounded quick/soak recipes, and report
  schema `hyperliquid-alpha-desk/state-replay-account-e2e-report/v1`.

- [ ] **Step 1: Write failing runner and CLI tests**

  Require:

  ```rust
  pub struct AccountRunConfig {
      pub output: PathBuf,
      pub blocks: u64,
      pub checkpoint_after: u64,
      pub iterations: u64,
  }

  pub fn run_account_e2e(
      config: &AccountRunConfig,
  ) -> Result<AccountEvidence, StateReplayError>;
  ```

  Assert at least two independent full replays, a checkpoint suffix that
  crosses a position reversal and funding event, identical final state/full
  receipt hashes, strict namespace counts, exact debit/credit symmetry,
  position/episode cardinalities, metadata-unresolved suppression, backstop
  unresolved-basis behavior, late-invalid atomic rollback, schema `1.1.0`
  denial, unsafe/existing output refusal, and recursive `0700`/`0600`
  permissions.

- [ ] **Step 2: Run red**

  ```bash
  cargo +1.97.1 test -p state-replay --test account_e2e --test cli --locked --offline
  ```

- [ ] **Step 3: Implement honest evidence**

  Use:

  ```text
  evidence_class = synthetic_canonical_account
  state_semantics = exact_cashflow_and_supported_position_state
  source_qualification = synthetic_unassessed
  reducer_version = hyperliquid-alpha-desk-canonical-state@1.0.0
  ```

  Only `synthetic_account_contract_proven` may be true. Explicitly keep Stage
  1, Stage 2, deployed/live source, authoritative opening balance, venue
  balance reconciliation, TWAP position completeness, backstop cost basis,
  standard/unified/portfolio margin, liquidation-price, book, signal, and
  execution qualification false.

- [ ] **Step 4: Add retained quick and soak recipes**

  ```bash
  just state-replay-account-e2e 30 12 4
  just state-replay-account-soak
  ```

  Both commands preserve the full evidence directory and refuse existing
  output. The soak recipe remains bounded by the shared replay-work validator.

- [ ] **Step 5: Run full gates and commit**

  ```bash
  cargo +1.97.1 test -p state-replay --locked --offline
  cargo +1.97.1 clippy -p state-replay --all-targets --all-features --locked --offline -- -D warnings
  just state-replay-account-e2e 30 12 4
  just deny
  RUST_TEST_THREADS=1 just verify
  just oss-audit
  git diff --check
  git add tools/state-replay justfile README.md docs
  git commit -m "feat(replay): add canonical account evidence runner"
  ```

## Plan Self-Review

- Spec coverage: all nineteen current V1 account/risk event kinds are typed;
  exact supported cash-flow, mode, ordinary-fill position, episode,
  liquidation, settlement, composite replay, rollback, checkpoint, and
  qualification behavior has an owning task.
- Explicit follow-on gaps: authoritative opening/account snapshots and source
  mapping, advanced TWAP/trigger schema sufficiency, versioned margin models,
  order book, and deployed-source reconciliation are separate plans because
  current V1 cannot prove their required inputs.
- Placeholder scan: no `TBD`, `TODO`, inferred protocol value, or unowned
  interface remains.
- Type consistency: addresses remain `Address`; protocol IDs remain typed;
  quote and asset amounts are not silently interchanged; fixed-point scale
  normalization is exact-only.

## Verification

- `cargo +1.97.1 test -p domain-types -p api-contracts -p canonical-events --locked --offline`
- `cargo +1.97.1 test -p canonical-ledger -p replay-engine --locked --offline`
- `cargo +1.97.1 test -p state-replay --locked --offline`
- strict Clippy with warnings denied for every affected crate
- `just state-replay-account-e2e 30 12 4`
- `just deny`
- `RUST_TEST_THREADS=1 just verify`
- `just oss-audit`

## Decision Log

- 2026-07-29: Cash-flow totals are not called balances because V1 carries no
  authoritative opening snapshot and deposit routing depends on account mode.
- 2026-07-29: Backstop liquidation changes position resolution to unresolved;
  V1 has no transfer price or replacement cost basis.
- 2026-07-29: TWAP fills do not mutate exact positions because V1
  `TwapStarted` omits side. A future schema must add sufficient semantics
  rather than infer direction from external state.
- 2026-07-29: Standard/unified/portfolio margin and liquidation-price formulas
  remain a separate versioned margin-model plan. Current official protocol
  documentation is discovery input, not a substitute for immutable reviewed
  rule fixtures and source snapshots.
- 2026-07-29: Preserve signed fee rates. A frozen `maker_rebate` is a credit
  with a negative rate; charged fee types require positive rates. Do not erase
  the documented rebate sign by coercing every fee rate nonnegative.
- 2026-07-30: Split account flows into `Quantity` and `QuoteAmount` record
  families with event-proven scopes. A single generic flow record or a
  catch-all “spot” scope would permit unit coercion or invent venue routing.
- 2026-07-30: A referral reward credits `referrer_account_id` only and never
  debits the primary account because the payload omits the protocol reward
  source. Builder fees retain explicit payer-to-builder symmetry. This matches
  Hyperliquid's documented
  [referral](https://hyperliquid.gitbook.io/hyperliquid-docs/referrals) and
  [builder-code](https://hyperliquid.gitbook.io/hyperliquid-docs/trading/builder-codes)
  recipient semantics while deployed-source mapping remains unqualified.
- 2026-07-30: A subaccount relation is derived only when the declared master
  is exactly one transfer endpoint. Three-distinct-account payloads remain
  valid typed facts but fail state reduction as relationship-ambiguous.
- 2026-07-30: Vault conservation means reconciliation of observed account and
  vault flow totals independently for quote principal and share quantity. It
  is not a principal/share equality, current holding, or active-membership
  claim.

## Progress Log

- 2026-07-29: Plan created after dynamic market registry whole-plan GO at
  `3514605`. Task 1 typed cash-flow contracts is next.
- 2026-07-29: Task 1 completed at `6af8feb`. The initial implementation
  promoted all seven cash-flow/transfer/vault payloads. Independent review
  held the slice until a shared 16 KiB limit covered encoder, direct decoder,
  canonical decode, and generic validation before Prost work; deterministic
  defaults and the complete independent negative matrix were also added.
  Final re-review returned GO. Parent verification passed 98 affected tests,
  strict all-target/all-feature Clippy, formatting, and diff checks.
- 2026-07-29: Task 2 completed at `ace439b`. Eight fee, funding, reward,
  account-mode, margin-mode, and leverage payloads are strict typed contracts.
  Maker rebates retain their negative signed rate and credit direction;
  charged fee types require positive rates. Independent review held only for
  incomplete boundary evidence, then returned GO after public encoder/direct
  decoder limits, all amount/identity fields, funding signs, and fixed-point
  precision 38/39 were covered. Parent verification passed 144 affected
  tests, strict all-target/all-feature Clippy, formatting, and diff checks.
- 2026-07-30: Task 3 completed at `81f2a2d`. All four liquidation and
  settlement payloads plus `LiquidationId` are strict typed contracts. Exact
  semantics cover liquidation margin ordering and scale, positive fills,
  distinct backstop accounts, zero-price settlement, and signed realized PnL
  without inventing backstop price or basis. Final independent review returned
  spec-compliance and code-quality GO. Worker, reviewer, and parent each
  reproduced the focused and 152-test package gates; strict Clippy, formatting,
  size preflight, and exact-range diff checks passed.

## Rollback / Recovery

- Retain `3514605` as the clean fallback before account payload promotion.
- If a payload lacks sufficient semantics, keep its exact typed fact and
  explicit unresolved state; never weaken validation or synthesize a balance,
  side, transfer price, cost basis, or margin result.
