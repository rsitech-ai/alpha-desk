# Typed Canonical Order Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace opaque V1 order payloads with strict typed canonical contracts and reconstruct an exact deterministic order lifecycle without inferring positions, venue roles, or deployed-source qualification.

**Architecture:** `canonical-events` owns byte-exact typed payload validation. `canonical-ledger` owns a default-deny reducer with immutable order facts, current lifecycle state, and stored transition assessments. Immutable archive/checkpoint replay proves repeatability using generated canonical events only.

**Tech Stack:** Rust 1.97.1, Prost canonical V1, exact fixed-point domain types, BLAKE3 state hashing, immutable Parquet archive, private local checkpoints.

## Global Constraints

- Stage 1 remains unsigned and Stage 2 remains unqualified; reports must say so.
- V1 remains read-only with no signer, private key, placement, or execution route.
- Support exact `1.0.0` order schemas only; unknown values fail closed.
- Preserve enclosing canonical envelope bytes containing valid unknown Protobuf fields.
- Do not infer maker/taker, buyer/seller, position, fee, funding, margin, or balance effects.
- A block commits every order mutation and assessment or none.
- Every task follows red-green-refactor and focused verification.

---

### Task 1: Type admission and modification payloads

**Files:**
- Modify: `crates/domain-types/src/shared.rs`
- Modify: `crates/domain-types/src/lib.rs`
- Modify: `crates/domain-types/tests/shared.rs`
- Modify: `crates/api-contracts/src/lib.rs`
- Modify: `crates/api-contracts/tests/payload_codec.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Create: `crates/canonical-events/tests/order_payloads.rs`

**Interfaces:**
- Consumes: V1 `OrderAccepted`, `OrderRested`, and `OrderModified` Protobuf.
- Produces: public typed payloads using `OrderId`, `Address`, `MarketId`,
  `OrderSide::{Buy,Sell}`, `Price`, and `Quantity`; exact wire encode/decode
  helpers remain owned by `api-contracts`.

- [x] **Step 1: Write failing contract tests**

  Assert exact round-trip; reject blank/padded IDs, unknown side values,
  non-positive prices/quantities, unchanged modifications, and kind mismatch.
  Assert an enclosing wire event with an unknown field remains byte-preserving.

- [x] **Step 2: Run the red tests**

  ```bash
  cargo +1.97.1 test -p canonical-events --test order_payloads --locked --offline
  ```

  Expected: compile failure because the typed payloads do not exist.

- [x] **Step 3: Implement strict codecs**

  Add `OrderSide::{Buy,Sell}` rather than reusing positional
  `Direction::{Long,Short,Flat}`. Add exact wire structs and decode/encode
  helpers in `api-contracts`. Remove only these three variants from
  `opaque_payloads!`; decode exact domain values; accept only lowercase `buy`
  and `sell`; encode deterministically; keep the existing enclosing-envelope
  forward-compatible byte path.

- [x] **Step 4: Verify**

  ```bash
  cargo +1.97.1 test -p domain-types -p api-contracts -p canonical-events --locked --offline
  cargo +1.97.1 clippy -p domain-types -p api-contracts -p canonical-events --all-targets --locked --offline -- -D warnings
  ```

- [x] **Step 5: Commit**

  ```bash
  git add crates/domain-types crates/api-contracts crates/canonical-events
  git commit -m "feat(events): type canonical order admission payloads"
  ```

---

### Task 2: Type fill and terminal payloads

**Files:**
- Modify: `crates/api-contracts/src/lib.rs`
- Modify: `crates/api-contracts/tests/payload_codec.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Modify: `crates/canonical-events/tests/order_payloads.rs`

**Interfaces:**
- Consumes: `OrderPartiallyFilled`, `OrderFilled`, `OrderCancelled`, and `OrderRejected`.
- Produces: typed exact identities, fill/remaining values, and bounded reasons.

- [x] **Step 1: Write failing semantic tests**

  Cover positive fill values, nonnegative remaining quantity, partial fill with
  zero remaining rejection, exact fill identity, bounded single-line reasons,
  strict rejection identities, unknown fields, and wrong event kinds.

- [x] **Step 2: Run the red tests**

  ```bash
  cargo +1.97.1 test -p canonical-events --test order_payloads --locked --offline
  ```

- [x] **Step 3: Implement the four typed codecs**

  Add exact wire encode/decode helpers in `api-contracts`. Remove only these
  variants from `opaque_payloads!`; parse exact identifiers and decimals; cap
  reason code at 128 bytes and reason at 1,024 bytes; reject controls and
  noncanonical re-encoding.

- [x] **Step 4: Verify and commit**

  ```bash
  cargo +1.97.1 test -p api-contracts -p canonical-events --locked --offline
  cargo +1.97.1 clippy -p api-contracts -p canonical-events --all-targets --locked --offline -- -D warnings
  git add crates/api-contracts crates/canonical-events
  git commit -m "feat(events): type canonical order outcome payloads"
  ```

---

### Task 3: Implement exact order lifecycle state

**Files:**
- Create: `crates/canonical-ledger/src/order.rs`
- Modify: `crates/canonical-ledger/src/lib.rs`
- Create: `crates/canonical-ledger/tests/order_state.rs`
- Modify: `docs/contracts/deterministic-state-v1.md`

**Interfaces:**
- Consumes: exact typed order payloads and envelope account/market identities.
- Produces: `CanonicalOrderReducerV1`, immutable facts, current order state, and stored transition assessments.

- [x] **Step 1: Write failing reducer tests**

  Prove `Accepted -> Rested -> PartiallyFilled -> Filled`,
  `Accepted -> Cancelled`, and rejection histories. Reject fill before
  acceptance, identity mismatch, overfill, inconsistent remaining quantity,
  modification after terminal state, collision, unsupported schema, and a late
  invalid event without changing pre-block bytes/hash.

- [x] **Step 2: Run red**

  ```bash
  cargo +1.97.1 test -p canonical-ledger --test order_state --locked --offline
  ```

- [x] **Step 3: Implement bounded key-bound records**

  Use namespaces `order-fact.v1`, `order-current.v1`, and
  `order-transition.v1`; strict canonical JSON capped at 16 KiB; market/order
  identity-bound keys; current accepted/filled/remaining quantities and exact
  lifecycle; transition records binding prior hash, event/payload identity,
  result hash, rule version, and status.

- [x] **Step 4: Implement default-deny transitions**

  Freeze `hyperliquid-alpha-desk-canonical-order@1.0.0`; own only the seven
  exact `1.0.0` kinds; require exact envelope identities; use checked
  fixed-point arithmetic; never resurrect terminal state. Rejections create
  immutable facts but no active order.

- [x] **Step 5: Verify and commit**

  ```bash
  cargo +1.97.1 test -p canonical-ledger --test order_state --locked --offline
  cargo +1.97.1 test -p replay-engine --locked --offline
  cargo +1.97.1 clippy -p canonical-ledger -p replay-engine --all-targets --locked --offline -- -D warnings
  git add crates/canonical-ledger docs/contracts/deterministic-state-v1.md
  git commit -m "feat(state): reconstruct canonical order lifecycle"
  ```

---

### Task 4: Add replay/checkpoint evidence and soak

**Files:**
- Refactor: `tools/state-replay/src/lib.rs`
- Create: `tools/state-replay/src/order.rs`
- Create: `tools/state-replay/tests/order_e2e.rs`
- Modify: `tools/state-replay/src/main.rs`
- Modify: `tools/state-replay/tests/cli.rs`
- Modify: `justfile`
- Modify: `README.md`
- Modify: `docs/STATUS.md`
- Modify: `docs/runbooks/state-replay-evidence.md`

**Interfaces:**
- Consumes: the order reducer, immutable archive, checkpoint store, serial replay.
- Produces: `state-replay order-e2e`, quick/soak recipes, and private report schema `hyperliquid-alpha-desk/state-replay-order-e2e-report/v1`.

- [ ] **Step 1: Write failing runner/CLI tests**

  Assert repeated/resumed hashes, exact fact/current/transition counts, terminal
  cardinality, malformed and unsupported atomic rejection, `0700`/`0600`
  permissions, and explicit false Stage 1/2, live, position, margin, and
  execution qualification.

- [ ] **Step 2: Run red**

  ```bash
  cargo +1.97.1 test -p state-replay --test order_e2e --test cli --locked --offline
  ```

- [ ] **Step 3: Modularize and implement**

  Split the current replay library into shared, fixture, archive, trade, and
  order modules, preserving existing public APIs and tests. Add the bounded
  order runner using shared output/archive/checkpoint/rejection primitives.

- [ ] **Step 4: Retained run and full gates**

  ```bash
  just state-replay-order-e2e 20 8 4
  just deny
  just verify
  just oss-audit
  ```

  Expected: local PASS without Stage 1/2 qualification.

- [ ] **Step 5: Commit**

  ```bash
  git add tools/state-replay justfile README.md docs/STATUS.md docs/runbooks/state-replay-evidence.md
  git commit -m "feat(replay): add canonical order evidence runner"
  ```

## Self-Review

- Coverage: typed V1 order contracts, exact lifecycle state, atomicity,
  replay/checkpoint evidence, and qualification boundaries are covered.
- Explicit gaps: positions, balances, fees, funding, margin, books,
  deployed-source mapping, and signed gates remain later work.
- Consistency: the same seven kinds, reducer version, namespaces, command, and
  report boundaries are used across tasks.
