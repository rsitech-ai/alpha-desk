# Hyperliquid Alpha Desk — Full Coverage Implementation Plan

> **For agentic workers:** execute this plan with an isolated worktree, test-driven development, one focused commit per task, requirement review, code-quality review, and verification evidence before claiming completion.

**Date:** 2026-08-19  
**Target repository:** `rsitech-ai/alpha-desk`  
**Design dependency:** `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md` plus the companion full-coverage expansion addendum.  
**Goal:** make Alpha Desk the read-only, evidence-preserving, deterministic system of record and intelligence layer for all current public Hyperliquid HyperCore and HyperEVM data, while reusing `hlscreen` without duplicating production authority.

---

## 1. Execution rules

### 1.1 Non-negotiable constraints

- Keep the existing five deployables: `hl-capture`, `hl-core`, `hl-analytics`, `hl-research`, `hl-api`.
- Add pure crates only when they create a real architecture boundary; do not create a deployable per domain.
- Raw evidence is durably archived before acknowledgement/publication.
- Canonical accounting and identity never use `f64`.
- Canonical reducers remain synchronous and deterministic.
- Live and replay use the same parser, mapping, reducer, feature, and signal paths.
- No `/exchange`, signing, execution, credentials, private keys, order placement, or copy trading.
- Official WebSocket/REST remain provisional/reconciliation sources; committed node evidence remains primary.
- Third-party providers never silently overwrite canonical truth.
- Unknown state-affecting variants are quarantined and fail the affected health gate.
- Every task starts with a failing test or failing compatibility check.
- Each PR updates coverage documentation, fixtures, and operational metrics relevant to its change.
- No stage or milestone advances on elapsed time alone.

### 1.2 Branch and worktree

Create an isolated worktree:

```bash
git fetch origin
git worktree add ../alpha-desk-full-coverage -b feature/hyperliquid-full-coverage origin/main
cd ../alpha-desk-full-coverage
```

Before each task:

```bash
git status --short
just check-workspace
```

After each task:

```bash
just verify
git status --short
```

Use focused Conventional Commit subjects, for example:

```text
feat(protocol): add manifest-driven info capability registry
feat(capture): schedule weighted wallet reconciliation calls
feat(core): reconcile committed account state against official snapshots
feat(analytics): project wallet episodes and cashflow-adjusted equity
```

### 1.3 Review gates per task

1. **Requirements review**
   - exact capability/domain is covered;
   - no duplicate product/runtime authority;
   - source role and limitations are explicit;
   - live/replay equivalence is preserved.

2. **Code-quality review**
   - deterministic IDs and ordering;
   - checked numeric conversions;
   - no hidden lossy parsing;
   - bounded memory/backpressure;
   - meaningful tests;
   - metrics and errors;
   - no execution/signing dependency.

---

## 2. Milestone map

| Milestone | Outcome | Hard gate |
|---|---|---|
| M0 — Baseline | approved addendum, current coverage snapshot, no architecture regression | spec/plan review |
| M1 — Capability ownership | every public source capability has manifest ownership, fixtures, and status | coverage check passes |
| M2 — Official realtime/snapshot | complete `/info` and WS adapters with budgets, archive-first semantics, and hlscreen parity | live qualification |
| M3 — Committed HyperCore | full node datasets, L4, historical backfill, canonical additions, reducers, reconciliation | deterministic replay |
| M4 — Wallet system | committed discovery, registry, tiered scheduling, history, current-state reconciliation | 10k-wallet qualification |
| M5 — HyperEVM | raw blocks/receipts/logs, system transactions, ABI registry, Core/EVM links | EVM replay and continuity |
| M6 — Intelligence runtime | complete ClickHouse projections and operational `hl-analytics` | projection rebuild |
| M7 — Product | wallet/market/ecosystem/EVM APIs, alerts, dashboard/ops views | API contract and UX qualification |
| M8 — Production hardening | load, soak, chaos, restore, cost, licensing, release policy | release gate |

---

## 3. Task dependency graph

```text
T01 -> T02 -> T03
T03 -> T04 -> T05 -> T06 -> T07 -> T08 -> T09
T03 -> T10 -> T11 -> T12
T03 -> T13 -> T14 -> T15

T09 + T12 + T15 -> T16 -> T17 -> T18 -> T19
T19 -> T20 -> T21 -> T22
T16 + T22 -> T23

T03 -> T24 -> T25 -> T26
T19 + T23 + T26 -> T27 -> T28 -> T29 -> T30 -> T31
T31 -> T32 -> T33 -> T34
T33 -> T36
T34 + T36 -> T37 -> T38 -> T39 -> T40

T03 + T16 -> T35  (optional-provider lane)
T35 -> T37/T39 only when a provider is enabled in the release profile
```

The optional-provider lane never blocks the own-node/official-source core unless an enabled release profile declares that provider mandatory.

---

# Phase A — Baseline and coverage governance

## T01 — Commit the full-coverage design addendum and traceability record

**Goal:** add the approved additive design without replacing the existing 2026-07-24 design.

**Files**

- Create `docs/superpowers/specs/2026-08-19-hyperliquid-full-coverage-expansion.md`
- Create `docs/superpowers/plans/2026-08-19-hyperliquid-full-coverage-plan.md`
- Create `docs/superpowers/plans/2026-08-19-hyperliquid-full-coverage-traceability.md`
- Update `docs/superpowers/plans/2026-07-24-00-hyperliquid-alpha-desk-program-roadmap.md` [absent on this branch; do not invent it; contributor roadmap is `docs/ROADMAP.md`]
- Update `docs/STATUS.md` with a dated note that it is a snapshot and runtime maturity differs by component

**Failing check first**

Add a documentation validation check that fails until:

- both new documents exist;
- they reference the approved base design;
- they explicitly preserve read-only scope;
- every requirement ID in the addendum appears in traceability.

**Implementation**

Assign requirement IDs:

```text
HLCOV-SRC-001...
HLCOV-PROTO-001...
HLCOV-CORE-001...
HLCOV-WALLET-001...
HLCOV-EVM-001...
HLCOV-ANALYTICS-001...
HLCOV-API-001...
HLCOV-OPS-001...
```

Trace each to planned task, target test, and acceptance evidence.

**Verification**

```bash
just check-workspace
just verify
```

**Commit**

```text
docs(design): add Hyperliquid full-coverage expansion
```

---

## T02 — Add the machine-readable Hyperliquid capability manifest

**Goal:** make “all supported public data” measurable and code-owned.

**Files**

- Create `config/hyperliquid/capabilities.toml`
- Create `schemas/hyperliquid/capability-manifest-v1.schema.json`
- Create `docs/hyperliquid/coverage-matrix.md`
- Create `tools/hyperliquid-capabilities/Cargo.toml`
- Create `tools/hyperliquid-capabilities/src/main.rs`
- Update root `Cargo.toml`
- Update `justfile`
- Add tests under `tools/hyperliquid-capabilities/tests/`

**Failing tests first**

- duplicate capability IDs fail;
- missing parser owner fails;
- missing fixture set fails for `implemented*` statuses;
- unsupported network requires a reason;
- state-affecting capabilities cannot be `opaque_continue`;
- generated matrix differs from committed matrix;
- every current node dataset, S3 dataset, `/info` endpoint family, and WS family is represented.

**Implementation**

The tool shall support:

```bash
hyperliquid-capabilities validate
hyperliquid-capabilities render-docs
hyperliquid-capabilities coverage
hyperliquid-capabilities diff --left report-a.json --right report-b.json
```

Add:

```make
hyperliquid-coverage-check:
    cargo +1.97.1 run -p hyperliquid-capabilities --locked --offline -- validate
    cargo +1.97.1 run -p hyperliquid-capabilities --locked --offline -- render-docs --check
```

Manifest fields include source, network, transport, request/subscription/dataset identifier, domain, source role, request cost, pagination, parser, fixture set, retention, freshness target, owner, state target, status, and limitations.

**Verification**

```bash
just hyperliquid-coverage-check
just verify
```

**Commit**

```text
feat(coverage): add manifest-driven Hyperliquid capability registry
```

---

## T03 — Add source catalog, evidence provenance, and provider policy

**Goal:** represent authority, reliability, licensing, and retention separately.

**Files**

- Create `schemas/postgres/0005_source_catalog.sql`
- Add `crates/hl-protocol/src/source_catalog.rs`
- Extend `crates/hl-protocol/src/source.rs`
- Extend `crates/storage-ports/src/lib.rs`
- Create `crates/storage-ports/src/source_catalog.rs`
- Add migration and repository tests
- Update `services/hl-capture/src/config.rs`
- Update `services/hl-api/src/openapi.rs` generation inputs as appropriate

**Failing tests first**

- committed-primary and discovery-only roles cannot be conflated;
- a provider source requires licensing and redistribution policy fields;
- source IDs are stable and network-scoped;
- a source cannot be marked committed without a qualifying evidence class;
- source record temporal updates preserve history;
- disabled/expired provider agreements suppress scheduled work.

**Schema**

Core tables:

```text
source_registry
source_capability_binding
source_endpoint_version
source_license_policy
source_health_policy
source_probe_result
```

Do not store provider secrets in PostgreSQL.

**Implementation**

Add domain types:

```rust
pub enum SourceRole {
    CommittedPrimary,
    CommittedIndependent,
    ProvisionalRealtime,
    ReconciliationSnapshot,
    HistoricalBackfill,
    AttributionEnrichment,
    DiscoveryOnly,
}

pub struct SourceDescriptor {
    pub source_id: SourceId,
    pub network: NetworkId,
    pub role: SourceRole,
    pub operator: String,
    pub dataset_version: Option<String>,
    pub retention_class: RetentionClass,
    pub redistribution: RedistributionPolicy,
}
```

**Verification**

```bash
just postgres-migration-smoke
cargo +1.97.1 test -p hl-protocol -p storage-ports --locked --offline
just verify
```

**Commit**

```text
feat(provenance): add source catalog and provider policy
```

---

# Phase B — hlscreen reuse and official protocol coverage

## T04 — Import hlscreen golden fixtures and differential test harness

**Goal:** reuse proven parsing/feature behavior without creating a runtime dependency or second truth system.

**Files**

- Create `fixtures/hyperliquid/hlscreen/README.md`
- Copy selected legally compatible raw fixtures into `fixtures/hyperliquid/hlscreen/`
- Create `crates/hl-protocol/tests/hlscreen_parser_parity.rs`
- Create `crates/feature-core/tests/hlscreen_feature_parity.rs`
- Create `docs/hyperliquid/hlscreen-reuse-map.md`
- Add a fixture-generation/export command to `hlscreen` later in T34; do not block this task

**Failing tests first**

For shared messages:

- spot metadata;
- spot/perp asset contexts;
- trades;
- BBO;
- L2;
- mids;
- candles;
- active asset context;
- reconnect snapshot sequences.

Tests fail until Alpha Desk can parse the raw fixture and produce an approved normalized semantic record.

**Implementation rules**

- No Git dependency on `hlscreen`.
- Preserve original fixture hash and source commit.
- Record intentional semantic differences, especially exact decimal parsing and source-evidence metadata.
- Feature parity may compare rational/fixed-point outputs or documented tolerance only for non-canonical analytics.

**Verification**

```bash
cargo +1.97.1 test -p hl-protocol --test hlscreen_parser_parity --locked --offline
cargo +1.97.1 test -p feature-core --test hlscreen_feature_parity --locked --offline
just verify
```

**Commit**

```text
test(protocol): add hlscreen differential fixture suite
```

---

## T05 — Build the manifest-driven `/info` protocol framework

**Goal:** create one exact, extensible request/response registry for all official read endpoints.

**Files**

- Create `crates/hl-protocol/src/info/mod.rs`
- Create `crates/hl-protocol/src/info/request.rs`
- Create `crates/hl-protocol/src/info/response.rs`
- Create `crates/hl-protocol/src/info/registry.rs`
- Create `crates/hl-protocol/src/info/pagination.rs`
- Update `crates/hl-protocol/src/lib.rs`
- Add fixtures under `fixtures/hyperliquid/official-info/`
- Add unit/property tests

**Failing tests first**

- capability manifest entry resolves to exactly one request encoder and response parser;
- decimals reject overflow and invalid scale;
- unknown fields preserve raw evidence and schema fingerprint;
- unknown state-affecting enum variants produce quarantine;
- pagination cursor handles identical timestamps;
- same request serializes deterministically;
- source observation hash is stable.

**Implementation**

Create typed but byte-preserving result:

```rust
pub struct ParsedInfoResponse<T> {
    pub capability_id: CapabilityId,
    pub request_hash: ContentHash,
    pub response_hash: ContentHash,
    pub schema_fingerprint: SchemaFingerprint,
    pub received_at: Timestamp,
    pub raw_archive_ref: ArchiveRef,
    pub value: T,
    pub unknown_fields: Vec<JsonPath>,
}
```

Do not couple the request registry to `reqwest`; transport stays in `hl-capture`.

**Verification**

```bash
cargo +1.97.1 test -p hl-protocol info --locked --offline
just hyperliquid-coverage-check
just verify
```

**Commit**

```text
feat(protocol): add typed info request and response registry
```

---

## T06 — Implement general, account, order, history, fee, and relationship `/info` families

**Goal:** cover the high-value general/user endpoint surface.

**Files**

- Create:
  - `crates/hl-protocol/src/info/general.rs`
  - `accounts.rs`
  - `orders.rs`
  - `twap.rs`
  - `fees_referrals.rs`
  - `builders_agents.rs`
- Add fixtures for:
  - `allMids`
  - `openOrders`
  - `frontendOpenOrders`
  - `userFills`
  - `userFillsByTime`
  - `historicalOrders`
  - `orderStatus`
  - `l2Book`
  - `candleSnapshot`
  - `portfolio`
  - `userNonFundingLedgerUpdates`
  - `userTwapSliceFills` and exposed history variants
  - `userVaultEquities`
  - `userRole`
  - `userRateLimit`
  - `userFees`
  - `referral`
  - `subAccounts`
  - multisig/extra-agent/approved-builder/abstraction queries

**Failing tests first**

- all documented order statuses parse, including cancellation/rejection reasons;
- role variants preserve relationship data;
- fill IDs and partial aggregation semantics are stable;
- user history cap/coverage metadata is emitted;
- master, subaccount, and agent addresses are not conflated;
- spot/perp coin remapping produces canonical market IDs.

**Implementation**

Represent every response as either:

- reference snapshot;
- reconciled snapshot;
- bounded history observation;
- direct stable-ID lookup.

Do not emit committed ledger transitions from these parsers.

**Verification**

```bash
cargo +1.97.1 test -p hl-protocol info:: --locked --offline
just hyperliquid-coverage-check
just verify
```

**Commit**

```text
feat(protocol): cover general and user info endpoints
```

---

## T07 — Implement perp, spot, outcome, vault, staking, aligned-quote, and borrow/lend `/info` families

**Goal:** cover the rest of the current official read surface.

**Files**

- Create:
  - `crates/hl-protocol/src/info/perpetuals.rs`
  - `spot.rs`
  - `outcomes.rs`
  - `vaults.rs`
  - `staking.rs`
  - `borrow_lend.rs`
- Extend market/asset IDs in `domain-types` or existing registry modules
- Add fixtures for all current capability-manifest entries

**Required families**

Perpetuals:

- DEX list and metadata;
- metadata and asset contexts;
- all-perp metadata;
- clearinghouse state per DEX;
- funding history/predictions;
- OI-cap state;
- active asset data;
- DEX limits/status;
- deploy auction;
- annotations/categories.

Spot/outcomes:

- spot metadata and contexts;
- spot clearinghouse state;
- deploy state and auction;
- token details;
- outcome metadata when network-supported;
- distinct outcome IDs and names.

Ecosystem:

- vault details/equities;
- delegator summary/delegations/history/rewards;
- validator statistics when exposed;
- aligned quote state;
- borrow/lend user state;
- single/all reserve state.

**Failing tests first**

- builder-deployed perp asset IDs;
- spot ID versus token ID;
- outcome encoding;
- network-specific availability;
- DEX-specific account state;
- decimals and quote/collateral mapping;
- reserve values and optional fields;
- unsupported network returns explicit capability status, not parser failure.

**Verification**

```bash
cargo +1.97.1 test -p hl-protocol --locked --offline
just hyperliquid-coverage-check
just verify
```

**Commit**

```text
feat(protocol): cover perp spot outcome and ecosystem info data
```

---

## T08 — Add weighted REST scheduler and egress budgets

**Goal:** safely schedule snapshots/history under official and provider limits.

**Files**

- Create `services/hl-capture/src/request_budget.rs`
- Create `services/hl-capture/src/info_scheduler.rs`
- Create `services/hl-capture/src/egress.rs`
- Extend `services/hl-capture/src/config.rs`
- Extend `services/hl-capture/src/operator.rs`
- Add metrics in `telemetry`
- Add API emulator tests under `services/hl-capture/tests/`

**Failing tests first**

- base and response-size weights;
- 70–80% safety envelope;
- P0/P1 work cannot be starved by backfill;
- 429 reduces budget and does not create retry storm;
- cancellation/shutdown returns leases;
- multiple owned egress IDs maintain independent budgets;
- anonymous proxy configuration is rejected;
- variable-cost endpoint reserves conservatively and reconciles actual cost;
- deterministic scheduling under fixed clock/seed.

**Implementation**

Priority queue keys:

```text
priority
deadline
risk_score
last_success
estimated_cost
stable_job_id
```

Implement token buckets with monotonic clocks, jittered backoff, circuit breakers, and explicit reserve capacity.

**Verification**

```bash
cargo +1.97.1 test -p hl-capture request_budget --locked --offline
cargo +1.97.1 test -p hl-capture info_scheduler --locked --offline
just verify
```

**Commit**

```text
feat(capture): add weighted info request scheduler
```

---

## T09 — Implement official `/info` capture adapter with archive-first semantics

**Goal:** connect typed protocol support to durable capture.

**Files**

- Create `services/hl-capture/src/adapters/info_rest.rs`
- Update `services/hl-capture/src/adapters/mod.rs`
- Extend `services/hl-capture/src/app.rs`
- Extend `services/hl-capture/src/coordinator.rs`
- Extend `services/hl-capture/src/config.rs`
- Extend archive/storage ports as necessary
- Add `tools/ci/info-capture-e2e.sh`
- Update `justfile`

**Failing integration tests first**

- response bytes are archived before parsed observation publication;
- crash after archive/before publication replays exactly once;
- HTTP timeout leaves resumable job state;
- parser quarantine keeps raw bytes;
- source/capability/request hashes are present;
- content-identical duplicate responses do not duplicate state effects;
- bounded body size and decompression protection;
- TLS/host policy rejects unapproved endpoints.

**Add command**

```bash
just info-capture-e2e
```

**Verification**

```bash
just info-capture-e2e
just capture-outage-e2e
just capture-failover-e2e
just verify
```

**Commit**

```text
feat(capture): ingest official info responses through immutable archive
```

---

## T10 — Implement complete WebSocket protocol types and parser registry

**Goal:** support every documented subscription family and snapshot/incremental semantics.

**Files**

- Create `crates/hl-protocol/src/ws/mod.rs`
- Create `subscription.rs`
- Create `message.rs`
- Create `snapshot.rs`
- Create domain files as needed under `crates/hl-protocol/src/ws/`
- Add fixtures under `fixtures/hyperliquid/official-ws/`
- Add parity fixtures from hlscreen

**Failing tests first**

Cover all manifest entries, currently:

- all mids;
- notification/web data;
- TWAP state/history/slices;
- clearinghouse/open orders;
- candles/L2/trades/BBO;
- order updates/user events;
- fills/funding/non-funding ledger;
- active asset context/data;
- spot state;
- all-DEX account/asset state.

Test:

- subscription ack;
- initial snapshot;
- duplicate snapshot;
- incremental update;
- unknown channel;
- unknown user-event variant;
- liquidation and non-user-cancel;
- stable subscription identity;
- deterministic parsing.

**Implementation**

The parser returns:

```rust
pub enum WsObservation {
    Ack(...),
    Snapshot(...),
    Incremental(...),
    Heartbeat(...),
    Unknown(...),
}
```

Unknown state-affecting payloads are quarantined.

**Verification**

```bash
cargo +1.97.1 test -p hl-protocol ws:: --locked --offline
just hyperliquid-coverage-check
just verify
```

**Commit**

```text
feat(protocol): support complete public websocket surface
```

---

## T11 — Add WebSocket connection lifecycle and subscription planner

**Goal:** turn protocol support into a bounded, resilient runtime.

**Files**

- Create `services/hl-capture/src/adapters/public_ws.rs`
- Create `services/hl-capture/src/subscription_plan.rs`
- Create `services/hl-capture/src/ws_session.rs`
- Extend app/coordinator/config/metrics
- Port approved lifecycle patterns from hlscreen
- Add mock WebSocket integration tests

**Failing tests first**

- no more than configured 10 connections;
- no more than 30 new connections/minute;
- no more than 1000 subscriptions;
- no more than 10 unique user addresses across user-specific subscriptions;
- reserved failover capacity;
- reconnect with deterministic jitter;
- ping/inactivity detection;
- snapshot duplication after reconnect;
- stale subscriptions become red;
- orderly unsubscribe/shutdown;
- no event loss between archive and fan-out under bounded backlog.

**Implementation**

Planner inputs:

- capability manifest;
- active markets and DEXes;
- wallet Tier-0 user slots;
- configured reserve;
- source health;
- per-subscription freshness target.

Planner output is deterministic and diffable.

**Verification**

```bash
cargo +1.97.1 test -p hl-capture subscription_plan --locked --offline
just public-ws-replay
just capture-soak duration=15m
just verify
```

**Commit**

```text
feat(capture): add bounded public websocket planner and lifecycle
```

---

## T12 — Integrate WebSocket provisional and reconciliation lanes

**Goal:** publish low-latency observations without corrupting committed truth.

**Files**

- Extend `services/hl-capture/src/committed_pipeline.rs`
- Create `services/hl-capture/src/provisional_pipeline.rs`
- Extend NATS subjects in `services/hl-capture/src/bus/subjects.rs`
- Extend health schemas/protos
- Extend `hl-core` input contracts
- Add replay tests

**Failing tests first**

- provisional event never advances committed watermark;
- committed counterpart confirms the provisional observation;
- unmatched provisional event expires;
- reconnect snapshot reconciles but does not double-count;
- a conflicting committed event wins and produces finding;
- red provisional source suppresses provisional-only features;
- existing subject strings remain stable.

**Subject changes**

Add only compatible V1 subjects, for example:

```text
hl.v1.snapshot.account
hl.v1.snapshot.market
hl.v1.snapshot.ecosystem
hl.v1.health.source
```

Do not rename existing subjects.

**Verification**

```bash
cargo +1.97.1 test -p hl-capture -p hl-core --locked --offline
just state-replay-e2e
just verify
```

**Commit**

```text
feat(capture): separate provisional and reconciled websocket lanes
```

---

# Phase C — Committed HyperCore and historical truth

## T13 — Complete node transaction, miscellaneous-event, and state-snapshot schemas

**Goal:** parse every qualified committed node dataset into exact source records.

**Files**

- Extend `crates/hl-protocol/src/node/v1.rs`
- Add modules under `crates/hl-protocol/src/node/` for:
  - `transaction.rs`
  - `misc.rs`
  - `state_snapshot.rs`
- Extend `services/hl-capture/src/adapters/node_files.rs`
- Extend `node_stream.rs`
- Add node fixtures by dataset/schema version
- Extend qualification tooling

**Failing tests first**

- every known action/misc variant parses;
- unknown state-affecting variant quarantines;
- block/transaction/event coordinates are stable;
- file rotation/truncation is detected;
- periodic state snapshot hashes are reproducible;
- source offset resume is exact;
- duplicate files/records are idempotent.

**Implementation**

Do not prematurely map all observations into canonical events. First preserve exact typed node records plus raw bytes; mapping belongs in `canonical-events::node_mapping`.

**Verification**

```bash
cargo +1.97.1 test -p hl-protocol node:: --locked --offline
just capture-e2e
just verify
```

**Commit**

```text
feat(protocol): cover committed node transaction and miscellaneous schemas
```

---

## T14 — Complete node trades, order statuses, and raw L4 book diffs

**Goal:** obtain venue-wide wallet/order/book evidence.

**Files**

- Extend `crates/hl-protocol/src/node/v1.rs`
- Add/extend:
  - `node/trade.rs`
  - `node/order_status.rs`
  - `node/raw_book_diff.rs`
- Extend capture adapters
- Extend `crates/canonical-events/src/node_mapping.rs`
- Add fixtures and mapping tests

**Failing tests first**

- trade contains buyer and seller and preserves starting positions, OID, TWAP ID, client ID;
- maker/taker derivation is explicit and tested;
- every documented order status reason maps or quarantines;
- new/update/remove L4 diff semantics;
- same-price time priority;
- trigger-order metadata;
- stable event IDs across replay;
- wallet discovery evidence is emitted for both sides and resting-order users.

**Verification**

```bash
cargo +1.97.1 test -p hl-protocol -p canonical-events --locked --offline
just state-replay-trade-e2e
just state-replay-order-e2e
just verify
```

**Commit**

```text
feat(node): map venue-wide trades orders and L4 diffs
```

---

## T15 — Implement deterministic full L4 reconstruction and L2 reconciliation

**Goal:** maintain the full per-order book and derive trustworthy depth.

**Files**

- Extend `crates/orderbook/src/*`
- Extend `crates/canonical-ledger/src/order.rs`
- Extend `crates/canonical-ledger/src/market.rs`
- Extend `crates/canonical-ledger/src/composite.rs`
- Add RocksDB state encoding
- Add `schemas/parquet/l4-checkpoints-v1.json`
- Add `tools/ci/l4-replay-e2e.sh`
- Update `justfile`

**Failing tests first**

- snapshot bootstrap plus diffs;
- add/update/remove;
- duplicates and reordered provisional inputs;
- committed order preserves FIFO at price;
- trigger orders remain distinguishable;
- derived L2 equals expected fixture;
- official L2 comparison permits only versioned aggregation/timing tolerances;
- checkpoint restore yields identical hash;
- memory remains bounded under synthetic high order count.

**Verification**

```bash
just l4-replay-e2e
just state-replay-order-soak
just state-replay-market-soak
just verify
```

**Commit**

```text
feat(orderbook): reconstruct deterministic L4 and reconcile L2
```

---

## T16 — Add official historical S3 backfill and object manifests

**Goal:** bootstrap and repair history without ad hoc scripts.

**Files**

- Create `services/hl-capture/src/adapters/historical_s3.rs`
- Create `services/hl-capture/src/historical_manifest.rs`
- Extend `crates/storage-ports/src/archive.rs`
- Extend `crates/storage-ports/src/capture_progress.rs`
- Add fixture S3 emulator
- Add `tools/ci/historical-backfill-e2e.sh`

**Datasets**

- official L2 snapshot archive;
- asset contexts;
- node fills-by-block;
- older fills/trades formats;
- explorer blocks;
- replica commands;
- HyperEVM bucket is implemented in T23 using shared manifest infrastructure.

**Failing tests first**

- requester-pays configuration;
- predictable key range enumeration;
- ETag/hash verification;
- missing object recorded as gap;
- resumable range;
- parser version and coverage range;
- duplicate object import;
- crash/restart;
- old/new dataset format selection.

**Verification**

```bash
just historical-backfill-e2e
just archive-verify
just verify
```

**Commit**

```text
feat(capture): add resumable official historical backfill
```

---

## T17 — Extend canonical events and snapshots to Hyperliquid V1.1

**Goal:** represent missing committed domains and reconciled state without breaking V1.

**Files**

- Extend `crates/canonical-events/src/lib.rs`
- Extend `crates/canonical-events/src/node_mapping.rs`
- Extend `schemas/proto/canonical/v1/events.proto`
- Create `schemas/proto/canonical/v1/snapshots.proto`
- Add `schemas/parquet/reconciled-snapshots-v1.json`
- Add `schemas/parquet/reference-snapshots-v1.json`
- Add `schemas/parquet/unknown-payloads-v1.json`
- Update generated baselines and compatibility checks
- Extend `services/hl-capture/src/bus/subjects.rs`

**Candidate event additions**

Implement only those backed by fixtures/source evidence:

- non-user cancel;
- internal/account-class transfers;
- vault creation/distribution/leader commission;
- rewards/spot genesis;
- staking lifecycle and validator reward/jail state;
- borrow/lend actions and reserve changes;
- Core/EVM transfer and system action;
- additional market/deployment lifecycle.

**Failing tests first**

- protobuf compatibility;
- old V1 event round-trip;
- V1.1 upcast/downstream unknown handling;
- event-kind subject routing is exhaustive;
- snapshot cannot be accidentally applied as ledger transition;
- stable IDs;
- corrected envelope references superseded ID.

**Verification**

```bash
cargo +1.97.1 test -p canonical-events --locked --offline
just generated
just archive-verify
just verify
```

**Commit**

```text
feat(events): add compatible Hyperliquid V1.1 events and snapshots
```

---

## T18 — Implement missing deterministic reducers and reconciliation invariants

**Goal:** reconstruct all newly covered HyperCore state.

**Files**

- Extend `crates/canonical-ledger/src/account/*`
- Extend `position/*`
- Extend `market.rs`
- Extend `order.rs`
- Extend `composite.rs`
- Add modules:
  - `vault.rs`
  - `staking.rs`
  - `borrow_lend.rs`
  - `relationships.rs`
- Extend `crates/canonical-state-store/src/lib.rs`
- Extend `crates/storage-ports/src/state_store.rs`
- Add replay fixtures and commands

**Failing tests first**

- duplicate/correction/replay;
- account/position/order invariants;
- vault share/equity transitions;
- staking queue/delegation/reward transitions;
- borrow/lend balances and reserve actions;
- relationship effective-time state;
- account/market snapshots reconcile without double counting;
- feature health red on unresolved material divergence;
- exact state hash after checkpoint restore.

**RocksDB changes**

Version column families and key encoding. Add migration/rebuild policy rather than in-place reinterpretation.

**Verification**

```bash
just state-replay-account-e2e
just state-replay-position-e2e
just state-replay-market-e2e
cargo +1.97.1 test -p canonical-ledger -p canonical-state-store --locked --offline
just verify
```

**Commit**

```text
feat(state): reduce vault staking borrow-lend and relationship events
```

---

## T19 — Assemble the production `hl-core` runtime

**Goal:** operationalize the mature libraries as a real service.

**Files**

- Expand `services/hl-core/src/lib.rs`
- Expand `services/hl-core/src/main.rs`
- Add:
  - `app.rs`
  - `config.rs`
  - `consumer.rs`
  - `state_runtime.rs`
  - `checkpoint.rs`
  - `reconciliation.rs`
  - `publisher.rs`
  - `health.rs`
- Add service integration tests
- Add systemd/container config under existing infra conventions

**Failing tests first**

- durable NATS consumer resume;
- event archive/state watermark alignment;
- crash after state write/before ack;
- checkpoint restore;
- provisional/reconciled input handling;
- state-delta publication;
- red health suppresses dependent publication;
- graceful shutdown;
- no async I/O inside reducers;
- bounded backlog and disk-pressure behavior.

**Verification**

```bash
cargo +1.97.1 test -p hl-core --locked --offline
just state-replay-archive-e2e <fixture args>
just capture-outage-e2e
just verify
```

**Commit**

```text
feat(core): assemble canonical state and reconciliation runtime
```

---

# Phase D — Wallet discovery, scheduling, and history

## T20 — Add wallet-registry domain and PostgreSQL schema

**Goal:** create a durable, explainable wallet control plane.

**Files**

- Create `schemas/postgres/0006_wallet_registry.sql`
- Create pure crate:
  - `crates/wallet-registry/Cargo.toml`
  - `src/lib.rs`
  - `src/model.rs`
  - `src/priority.rs`
  - `src/coverage.rs`
  - `src/policy.rs`
- Update root `Cargo.toml`
- Create `crates/storage-ports/src/wallet_registry.rs`
- Add PostgreSQL adapter in the established service/storage location

**Failing tests first**

- address/network canonicalization;
- temporal role/relationship versions;
- multiple discovery reasons;
- deterministic priority score;
- tier transition hysteresis;
- exclusion and retention policies;
- coverage merge;
- refresh lease ownership/expiry;
- no provider label overwrites protocol role.

**Schema**

Include:

```text
wallet_registry
wallet_discovery_evidence
wallet_tracking_policy
wallet_source_coverage
wallet_label_version
wallet_backfill_cursor
wallet_refresh_lease
wallet_watchlist_membership
```

Reuse existing entity annotations where appropriate.

**Verification**

```bash
just postgres-migration-smoke
cargo +1.97.1 test -p wallet-registry -p storage-ports --locked --offline
just verify
```

**Commit**

```text
feat(wallets): add durable wallet registry and tracking policy
```

---

## T21 — Discover wallets from committed global evidence

**Goal:** remove leaderboard-only discovery bias.

**Files**

- Add `services/hl-analytics/src/discovery.rs`
- Add `services/hl-analytics/src/wallet_registry_runtime.rs`
- Extend canonical event consumers
- Add discovery evidence projections
- Add metrics

**Inputs**

- both trade participants;
- order-status/raw-L4 users;
- transaction actions;
- ledger and liquidation participants;
- vault/staking relationships;
- builder/agent/subaccount/multisig/referral relationships;
- Core/EVM links after T25.

**Failing tests first**

- both trade sides discovered once;
- repeated activity updates last-seen without losing first-seen;
- reason/evidence accumulation;
- deterministic priority changes;
- a provider-only wallet is marked provider-only until committed observation;
- entity/vault/agent addresses get correct roles;
- high-volume synthetic stream remains bounded.

**Verification**

```bash
cargo +1.97.1 test -p hl-analytics wallet_discovery --locked --offline
just analytics-projection-e2e
just verify
```

**Commit**

```text
feat(analytics): discover wallets from committed venue activity
```

---

## T22 — Implement tiered wallet refresh scheduling

**Goal:** support thousands of wallets under bounded budgets.

**Files**

- Create `services/hl-analytics/src/wallet_scheduler.rs`
- Create `services/hl-analytics/src/refresh_policy.rs`
- Extend `hl-capture` job intake/control subject
- Extend PostgreSQL leases/cursors
- Add deterministic scheduler tests and simulation

**Failing tests first**

- Tier 0–5 cadence;
- recent committed activity promotes;
- dormancy demotes with hysteresis;
- liquidation proximity and open notional affect urgency;
- official WS unique-user slots never exceed limit;
- REST estimated cost respects budget;
- overdue jobs prioritize fairly;
- provider batch endpoint is used only when configured and licensed;
- service restart preserves due jobs;
- 10,000-wallet synthetic simulation meets lateness targets.

**Output**

Scheduler emits source-agnostic refresh intents. `hl-capture` chooses official/provider adapters under source policy.

**Verification**

```bash
cargo +1.97.1 test -p hl-analytics wallet_scheduler --locked --offline
cargo +1.97.1 run -p hl-analytics --locked --offline -- simulate-wallet-scheduler --wallets 10000
just verify
```

**Commit**

```text
feat(wallets): schedule risk-aware wallet refresh tiers
```

---

## T23 — Implement wallet history backfill, coverage, and current-state reconciliation

**Goal:** produce reliable wallet histories and expose limitations.

**Files**

- Create `schemas/postgres/0007_backfill_jobs_and_coverage.sql`
- Create `services/hl-analytics/src/wallet_backfill.rs`
- Create `services/hl-analytics/src/wallet_reconciliation.rs`
- Extend `hl-capture` backfill job handling
- Extend wallet registry/storage ports
- Add `tools/ci/wallet-backfill-e2e.sh`
- Update `justfile`

**Backfill sequence**

- role/relationships;
- current perp/spot/all-DEX state;
- open/frontend orders and TWAP;
- fills by time;
- funding;
- non-funding ledger;
- historical orders;
- TWAP fills/history;
- portfolio observation;
- fee/referral;
- vault/staking/borrow-lend;
- node/S3/provider extension.

**Failing tests first**

- same-millisecond pagination;
- official 2k/10k history caps recorded;
- earliest reliable time by dataset;
- resumable cursors;
- duplicate pages;
- current-state reconciliation;
- no synthetic transition double counting;
- feature gating under incomplete coverage;
- provider-extension provenance.

**Verification**

```bash
just wallet-backfill-e2e
just wallet-reconciliation-e2e
just verify
```

**Commit**

```text
feat(wallets): backfill histories and reconcile current state
```

---

# Phase E — HyperEVM and cross-layer intelligence

## T24 — Add canonical HyperEVM block, transaction, receipt, and log types

**Goal:** create exact chain-fact contracts without pulling arbitrary ABI events into HyperCore canonical enums.

**Files**

- Create:
  - `crates/hl-protocol/src/evm/mod.rs`
  - `block.rs`
  - `transaction.rs`
  - `receipt.rs`
  - `log.rs`
  - `system_transaction.rs`
  - `asset.rs`
  - `core_link.rs`
  - `precompile.rs`
- Add dependencies only after architecture/security review
- Create Parquet schemas:
  - `evm-blocks-v1.json`
  - `evm-transactions-v1.json`
  - `evm-receipts-v1.json`
  - `evm-logs-v1.json`
- Add fixtures from mainnet/testnet raw examples

**Failing tests first**

- MessagePack/LZ4 fixture decoding;
- chain IDs 999/998;
- block/parent identity;
- transaction and log IDs;
- decimal/unit conversion;
- receipt status;
- unknown typed transaction fields preserved;
- system transaction representation;
- deterministic serialization/hash;
- no dependence on latest-RPC state for historical fact parsing.

**Verification**

```bash
cargo +1.97.1 test -p hl-protocol evm:: --locked --offline
just generated
just verify
```

**Commit**

```text
feat(protocol): add canonical HyperEVM chain fact types
```

---

## T25 — Implement local and S3 HyperEVM capture with continuity

**Goal:** archive committed HyperEVM data robustly.

**Files**

- Create:
  - `services/hl-capture/src/adapters/evm_local.rs`
  - `evm_s3.rs`
  - `evm_rpc.rs`
- Reuse historical manifest infrastructure
- Extend capture config/progress/health
- Add `tools/ci/hyperevm-replay-e2e.sh`
- Update `justfile`

**Failing tests first**

- local primary plus S3 fallback;
- requester-pays;
- predictable block-key layout;
- LZ4/MessagePack decode;
- parent-hash and block-number gap detection;
- duplicate block;
- conflicting block hash;
- testnet starting-gap metadata;
- crash/restart;
- official RPC rate budget;
- system transaction query;
- no reliance on unsupported historical-state methods.

**Verification**

```bash
just hyperevm-replay-e2e
just capture-outage-e2e
just verify
```

**Commit**

```text
feat(capture): ingest committed HyperEVM blocks with S3 fallback
```

---

## T26 — Add EVM asset decoding, ABI registry, system actions, and Core links

**Goal:** turn raw EVM facts into useful, evidence-linked intelligence.

**Files**

- Create `schemas/postgres/0008_contract_abi_and_provider_registry.sql`
- Add:
  - `services/hl-analytics/src/evm_indexer.rs`
  - `evm_decoders.rs`
  - `contract_registry.rs`
  - `core_evm_linker.rs`
- Extend `entity-graph`
- Extend canonical snapshot/cross-link types
- Add fixtures for ERC-20/721/1155, contract creation, system tx, CoreWriter, transfers

**Failing tests first**

- native transfer;
- ERC-20 transfer/approval;
- ERC-721/1155;
- mint/burn;
- proxy/implementation temporal version;
- unknown log retained;
- ABI change does not rewrite prior interpretation without new knowledge time;
- Core/EVM transfer amount conservation;
- system transaction links;
- CoreWriter known action decode;
- precompile observation links to EVM block and HyperCore height.

**Verification**

```bash
cargo +1.97.1 test -p hl-analytics evm_ --locked --offline
just cross-layer-reconciliation-e2e
just verify
```

**Commit**

```text
feat(evm): decode assets contracts and HyperCore links
```

---

# Phase F — Fact projections and analytics runtime

## T27 — Add complete ClickHouse fact schemas

**Goal:** create rebuildable analytical facts beneath existing features.

**Files**

- Create:
  - `schemas/clickhouse/0009_core_market_facts.sql`
  - `0010_order_account_ledger_facts.sql`
  - `0011_wallet_performance_and_rankings.sql`
  - `0012_vault_staking_borrow_lend.sql`
  - `0013_hyperevm_facts.sql`
  - `0014_reconciliation_alerts_and_coverage.sql`
- Add schema validation/migration tests
- Add typed projection row structs

**Failing tests first**

- exact decimal representation;
- primary/order/partition keys;
- dedupe/version semantics;
- as-of and knowledge-time columns;
- evidence ID/raw archive ref;
- rebuild ordering;
- no raw JSON-only table;
- TTL cannot delete sole evidence;
- migration smoke on clean and existing database.

**Verification**

```bash
# add/update the repository's ClickHouse migration smoke command
just analytics-projection-e2e
just verify
```

**Commit**

```text
feat(storage): add complete HyperCore and HyperEVM fact schemas
```

---

## T28 — Build deterministic archive-to-ClickHouse projectors

**Goal:** populate facts using replayable code paths.

**Files**

- Add projectors under `services/hl-analytics/src/projectors/`
  - `market.rs`
  - `orders.rs`
  - `accounts.rs`
  - `positions.rs`
  - `ledger.rs`
  - `vaults.rs`
  - `staking.rs`
  - `borrow_lend.rs`
  - `evm.rs`
  - `coverage.rs`
- Extend storage ports
- Add projector checkpoints and rebuild CLI

**Failing tests first**

- same archive produces same rows/hashes;
- crash after insert/before checkpoint;
- idempotent replay;
- correction/version handling;
- point-in-time fields;
- partition-boundary ordering;
- exact source evidence;
- projection lag/health;
- full database rebuild from fixture archive.

**CLI**

```bash
hl-analytics rebuild --domain all --archive <path> --from <height> --to <height>
hl-analytics verify-projection --expected-manifest <path>
```

**Verification**

```bash
just analytics-projection-e2e
cargo +1.97.1 test -p hl-analytics projectors --locked --offline
just verify
```

**Commit**

```text
feat(analytics): project canonical archives into analytical facts
```

---

## T29 — Assemble the production `hl-analytics` runtime

**Goal:** turn the stub deployable into the operational intelligence service.

**Files**

- Expand `services/hl-analytics/src/lib.rs`
- Expand `main.rs`
- Add:
  - `app.rs`
  - `config.rs`
  - `consumer.rs`
  - `projector_runtime.rs`
  - `wallet_runtime.rs`
  - `feature_runtime.rs`
  - `ranking_runtime.rs`
  - `alert_runtime.rs`
  - `health.rs`
- Add service integration tests and deployment config

**Failing tests first**

- durable consumer;
- projector checkpoints;
- wallet schedule lease;
- feature recompute after corrected evidence;
- red health suppresses affected feature/ranking/alert;
- bounded queues;
- ClickHouse outage backlog;
- PostgreSQL outage behavior;
- graceful restart;
- deterministic clock/seed where required.

**Verification**

```bash
cargo +1.97.1 test -p hl-analytics --locked --offline
just analytics-projection-e2e
just verify
```

**Commit**

```text
feat(analytics): assemble projection and intelligence runtime
```

---

## T30 — Productionize wallet episodes, equity, performance, and rankings

**Goal:** use existing wallet-intelligence code on complete facts.

**Files**

- Extend `crates/wallet-intelligence/src/*`
- Add `services/hl-analytics/src/wallet_features.rs`
- Add `wallet_rankings.rs`
- Extend ClickHouse projections/API contracts
- Add research/PIT tests

**Failing tests first**

- position open/add/reduce/close/flip;
- fees/funding/cashflow decomposition;
- cashflow-adjusted equity;
- max drawdown;
- episode win rate and profit factor;
- minimum sample and coverage;
- entity-independence weighting;
- ranking as-of reproducibility;
- provider-only history downgrade;
- copyability/capacity after spread, depth, latency, fees, impact.

**Outputs**

Rankings expose raw and adjusted metrics rather than one opaque score. Any composite score has versioned components and explanation.

**Verification**

```bash
cargo +1.97.1 test -p wallet-intelligence --locked --offline
cargo +1.97.1 test -p hl-analytics wallet_ --locked --offline
just verify
```

**Commit**

```text
feat(wallets): project cashflow-adjusted performance and rankings
```

---

## T31 — Integrate market, ecosystem, and hlscreen-derived features

**Goal:** feed existing market intelligence and port useful hlscreen features under one schema.

**Files**

- Extend `crates/feature-core/src/*`
- Extend `crates/market-intelligence/src/*`
- Add approved formulas from:
  - hlscreen microstructure;
  - resilience;
  - tradeability;
  - composite confidence;
- Add `services/hl-analytics/src/market_features.rs`
- Add ecosystem features for vaults/staking/borrow-lend/EVM
- Update feature schemas and parity tests

**Failing tests first**

- hlscreen parity or approved difference;
- no look-ahead;
- tracked versus global coverage labels;
- crowding denominator;
- liquidation-fragility inputs;
- funding/OI cap;
- vault/validator/reserve concentration;
- EVM flow;
- health suppression;
- deterministic feature set version.

**Verification**

```bash
cargo +1.97.1 test -p feature-core -p market-intelligence --locked --offline
cargo +1.97.1 test -p hl-analytics market_features --locked --offline
just verify
```

**Commit**

```text
feat(intelligence): unify market ecosystem and hlscreen features
```

---

# Phase G — Alerts, API, and product surfaces

## T32 — Implement deterministic alert rules and lifecycle

**Goal:** produce evidence-backed alerts without introducing a second signal system.

**Files**

- Create `schemas/postgres/0009_watchlists_and_alert_rules.sql`
- Extend `crates/signal-core/src/*` with alert-specific contracts or create a submodule, not a new deployable
- Add `services/hl-analytics/src/alert_runtime.rs`
- Add delivery outbox tables
- Add `tools/ci/alert-lifecycle-e2e.sh`
- Update `justfile`

**Failing tests first**

- rule versioning;
- point-in-time predicate evaluation;
- candidate/provisional/confirmed/reconciled/resolved/retracted/expired;
- deduplication;
- cooldown and hysteresis;
- corrected/retracted source event;
- red data health blocks confirmation;
- evidence bundle completeness;
- outbox idempotency and retry;
- no LLM decision in threshold crossing.

**Initial rules**

- large position open/add/reduce/close/flip;
- liquidation/backstop;
- funding/OI cap;
- liquidation distance;
- top-wallet direction change;
- crowding shift;
- vault drawdown;
- reserve stress;
- validator jail/concentration;
- Core/EVM transfer;
- source gap/divergence.

**Verification**

```bash
just alert-lifecycle-e2e
cargo +1.97.1 test -p signal-core -p hl-analytics alert --locked --offline
just verify
```

**Commit**

```text
feat(alerts): add evidence-backed deterministic alert lifecycle
```

---

## T33 — Expand OpenAPI and resumable streams

**Goal:** serve the complete intelligence surface with exact evidence metadata.

**Files**

- Extend `crates/api-contracts/src/*`
- Extend `services/hl-api/src/http.rs`
- Extend `services/hl-api/src/snapshot.rs`
- Extend OpenAPI generator and committed schema
- Add route modules:
  - `markets.rs`
  - `wallets.rs`
  - `ecosystem.rs`
  - `evm.rs`
  - `coverage.rs`
  - `alerts.rs`
- Extend stream contracts/resume tokens
- Add contract tests

**Failing tests first**

- exact decimal JSON;
- `as_of` and `knowledge_time`;
- watermark and confirmation;
- source coverage and data health;
- stable pagination;
- resume without sequence regression;
- stale cache marker;
- evidence lookup;
- role-based control writes;
- provider redistribution policy filters fields.

**Verification**

```bash
cargo +1.97.1 test -p api-contracts -p hl-api --locked --offline
just generated
just verify
```

**Commit**

```text
feat(api): expose complete wallet market ecosystem and EVM intelligence
```

---

## T34 — Add dashboard and operator workspaces

**Goal:** make the new evidence usable without hiding health/coverage.

**Files**

Depending on current product direction:

- extend `apps/AlphaDesk/` SwiftUI scenes/views;
- extend `apps/web-dashboard/` for internal development/ops;
- add shared API models generated from OpenAPI;
- add source/coverage/reconciliation panels;
- add UI snapshot and interaction tests.

**Required workspaces**

- market;
- wallet;
- ecosystem;
- HyperEVM;
- alerts;
- operations/data health.

**Failing tests first**

- stale data visibly marked;
- source and coverage disclosure;
- exact decimal formatting;
- stream resume;
- wallet/entity relation navigation;
- evidence drilldown;
- alert retraction display;
- accessibility labels and keyboard navigation;
- large-table virtualization/performance;
- no cached state shown as current after reconnect.

**Verification**

```bash
swift test --package-path apps/AlphaDesk
npm --prefix apps/web-dashboard test
just verify
```

**Commit**

```text
feat(desk): add full-coverage intelligence workspaces
```

---

# Phase H — Optional provider adapters and hlscreen client mode

## T35 — Add provider adapters behind source and licensing policy

**Goal:** add scalable history, attribution, wildcard streams, batching, and traces without vendor lock-in.

**Files**

- Create provider-neutral traits in `crates/storage-ports` or `hl-protocol`
- Add adapters under `services/hl-capture/src/adapters/providers/`
- Add provider config schemas
- Extend source catalog/provider registry
- Add contract tests with recorded fixtures
- Never commit credentials

**Potential adapters**

- Nansen: labels, positions, trades, leaderboards, smart-money discovery;
- QuickNode: HyperCore historical/streaming and HyperEVM archive/traces;
- GoldRush: batch wallet state, wildcard market/L4/wallet streams, warehouse/backfill;
- Allium: fills/orders/funding/non-funding/TWAP streams and history.

**Failing tests first**

- provider provenance on every field;
- provider disagreement never overwrites committed truth;
- licensing/redistribution enforcement;
- plan/capability change;
- pagination and history-gap metadata;
- provider outage;
- cost budget;
- schema drift;
- independent cross-check reports.

**Verification**

```bash
cargo +1.97.1 test -p hl-capture providers --locked --offline
just hyperliquid-coverage-check
just verify
```

**Commit**

```text
feat(providers): add policy-bound Hyperliquid enrichment adapters
```

---

## T36 — Add Alpha Desk API source mode to hlscreen

**Goal:** preserve hlscreen as a lightweight terminal while eliminating duplicate production capture.

**Target repository:** `rsitech-ai/hlscreen`

**Files**

- Add an `alpha-desk` source adapter to `hls-hyperliquid` or a new neutral client crate
- Add CLI configuration:
  - `--source official`
  - `--source alpha-desk`
- Map Alpha Desk market/state/feature/health streams into hlscreen’s view model
- Add contract fixtures from Alpha Desk OpenAPI/stream schemas
- Document standalone versus connected mode

**Failing tests first**

- exact stream resume;
- stale/health propagation;
- market ID mapping;
- no double recorder in connected mode;
- feature version display;
- fallback policy is explicit, never silent.

**Acceptance**

Production deployment uses Alpha Desk source mode. Standalone official mode remains available for local/OSS usage and qualification.

**Verification**

Use hlscreen’s full test/qualification suite plus Alpha Desk contract fixtures.

**Commit**

```text
feat(source): add Alpha Desk connected mode
```

---

# Phase I — Hardening and release

## T37 — Add full-coverage observability and source-health policy

**Goal:** prove what is fresh, complete, and degraded.

**Files**

- Extend `telemetry`
- Extend health protobufs and API
- Add dashboards/alerts under infra conventions
- Add source-capability metrics
- Add coverage snapshots

**Required metrics**

- source heights/lag/gaps;
- REST weights and scheduler lateness;
- WS connections/subscriptions/unique users/reconnects;
- raw archive throughput/latency;
- parser unknowns/quarantine;
- canonical/reducer/projector lag;
- reconciliation divergence;
- wallet coverage and refresh lateness;
- EVM continuity and decoder health;
- alert lifecycle/delivery;
- storage/cost.

**Failing tests first**

- red/yellow/green policy;
- dependent feature suppression;
- stale capability report;
- unknown variant pages operator;
- health survives restart;
- API includes affected domains.

**Verification**

```bash
cargo +1.97.1 test -p telemetry -p hl-api -p hl-analytics --locked --offline
just verify
```

**Commit**

```text
feat(observability): expose source coverage and dependency health
```

---

## T38 — Add capacity, soak, chaos, restore, and cost qualification

**Goal:** qualify the target scale and failure modes.

**Files**

- Add/update scripts under `tools/ci/`
- Add synthetic generators under `tools/`
- Add `config/qualification/full-coverage.toml`
- Add evidence report schemas
- Update `justfile`

**Scenarios**

- 100 GB/day equivalent raw node flow;
- all markets/DEXes and full L4;
- 10,000 wallet registry;
- thousands of refresh jobs;
- multi-year backfill;
- EVM blocks/logs;
- liquidation burst;
- node/API/provider/NATS/Postgres/ClickHouse outage;
- disk pressure;
- archive restore and ClickHouse rebuild.

**Commands**

```bash
just full-coverage-soak
just full-coverage-chaos
just full-coverage-restore
just full-coverage-cost-report
```

**Acceptance**

- no silent event loss;
- bounded memory and disk;
- deterministic hashes;
- documented SLO results;
- recovery point/recovery time evidence;
- storage and provider cost model;
- limitations recorded.

**Commit**

```text
test(qualification): add full-coverage load chaos and restore gates
```

---

## T39 — Enforce read-only release and provider-license policy

**Goal:** prevent scope drift and unsafe distribution.

**Files**

- Extend `config/open-source-policy.toml`
- Extend architecture/security checks
- Add dependency denylist for signing/execution crates in release graph
- Add endpoint denylist tests
- Add provider field-level redistribution checks
- Add release manifest scan
- Add threat-model addendum

**Failing checks first**

- `/exchange` string/route in production code;
- signing/private-key dependencies;
- secret-like fixture;
- provider field exposed without permission;
- notification adapter has access to capture/core secrets;
- execution binary in package/container/systemd manifests.

**Verification**

```bash
just architecture
just oss-audit
just deny
just verify
```

**Commit**

```text
chore(policy): enforce read-only and provider redistribution boundaries
```

---

## T40 — Run final full-coverage release gate

**Goal:** produce machine-verifiable evidence that the expansion is complete.

**Files**

- Create `config/stage-gates/hyperliquid-full-coverage.toml`
- Create `docs/stage-gates/hyperliquid-full-coverage.md`
- Add gate command to `justfile`
- Store canonical JSON evidence under the repository’s established evidence-copy convention

**Gate inputs**

- capability report;
- live qualification;
- replay hashes;
- source continuity;
- REST/WS budgets;
- L4 reconciliation;
- 10k-wallet simulation;
- EVM continuity;
- projection rebuild;
- alert lifecycle;
- API compatibility;
- load/soak/chaos/restore;
- read-only/security;
- provider licensing;
- independent review.

**Verification**

```bash
just hyperliquid-full-coverage-gate <builder_id>
```

The gate passes only if every mandatory capability is:

- `qualified_live` and/or `qualified_replay` as defined;
- explicitly unsupported by network with evidence;
- or explicitly source-unavailable with an approved limitation.

No vague “mostly complete” status is allowed.

**Commit**

```text
docs(gate): record Hyperliquid full-coverage qualification
```

---

## 4. Required new/updated commands

Add these incrementally:

```text
just hyperliquid-coverage-check
just info-capture-e2e
just public-api-fixtures
just public-ws-replay
just l4-replay-e2e
just historical-backfill-e2e
just wallet-backfill-e2e
just wallet-reconciliation-e2e
just analytics-projection-e2e
just hyperevm-replay-e2e
just cross-layer-reconciliation-e2e
just alert-lifecycle-e2e
just full-coverage-soak
just full-coverage-chaos
just full-coverage-restore
just full-coverage-cost-report
just hyperliquid-full-coverage-gate
```

All commands must be deterministic where they use fixtures and must write machine-readable reports under `target/evidence/`.

---

## 5. Definition of done per PR

A task is complete only when:

- failing test/check was committed or demonstrated first;
- implementation passes task tests;
- `just verify` passes;
- capability manifest status is updated;
- coverage matrix is regenerated;
- fixtures are hashed and documented;
- metrics and operator errors exist;
- raw/canonical/reconciled role is explicit;
- replay and live paths share code;
- no unreviewed `unsafe`;
- no execution/signing surface;
- requirement review passes;
- code-quality review passes;
- commit is focused and clean.

---

## 6. Program completion checklist

- [ ] Base design remains authoritative and the addendum is approved.
- [ ] Capability manifest covers official REST, WS, node, historical S3, HyperEVM, and configured provider sources.
- [ ] Every implemented capability has parser ownership and fixtures.
- [ ] Official REST/WS adapters are live-qualified.
- [ ] Full node trade/order/L4/misc data is archived and replayable.
- [ ] Historical backfill and gap manifests work.
- [ ] V1.1 canonical events/snapshots are backward compatible.
- [ ] All new reducers pass deterministic replay.
- [ ] `hl-core` is a real production runtime.
- [ ] Wallets are discovered from committed global evidence.
- [ ] 10,000-wallet scheduling is qualified.
- [ ] Wallet history and state reconciliation expose coverage.
- [ ] HyperEVM blocks/receipts/logs/system transactions are archived.
- [ ] Core/EVM transfers and known actions are linked.
- [ ] Complete ClickHouse fact schemas and rebuildable projectors exist.
- [ ] `hl-analytics` is a real production runtime.
- [ ] Wallet rankings are cashflow-adjusted, PIT-safe, entity-aware, and coverage-labelled.
- [ ] Market/ecosystem features use complete production facts.
- [ ] Alert lifecycle is deterministic and evidence-backed.
- [ ] API and streams expose watermarks, health, and evidence.
- [ ] Dashboard surfaces wallet/order/liquidation/vault/staking/borrow-lend/EVM intelligence.
- [ ] Provider adapters are policy-bound and optional.
- [ ] hlscreen connected mode prevents duplicate production capture.
- [ ] Load, soak, chaos, restore, cost, and security gates pass.
- [ ] Release contains no execution or signing capability.

---

## 7. Recommended first implementation slice

The first independently valuable slice is T01–T12:

1. commit the addendum and traceability;
2. add the capability manifest;
3. add source provenance;
4. import hlscreen fixtures;
5. implement complete `/info` protocol support;
6. implement weighted scheduling;
7. implement complete WebSocket parsing/lifecycle;
8. connect both to archive-first provisional/reconciliation lanes.

This slice produces immediate value without waiting for every downstream analytical projection:

- Alpha Desk gains official real-time and snapshot coverage;
- current source gaps become visible;
- hlscreen functionality is reused safely;
- future endpoint/schema drift becomes observable;
- the existing canonical/replay architecture remains intact.

The next slice should be T13–T19, because complete committed node evidence and `hl-core` runtime are prerequisites for trustworthy wallet rankings and “all-wallet” intelligence.
