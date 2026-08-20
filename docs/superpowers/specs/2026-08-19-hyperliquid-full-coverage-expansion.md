# Hyperliquid Alpha Desk — Full Public-Data Coverage Expansion

**Status:** Proposed design addendum  
**Date:** 2026-08-19  
**Target repository:** `rsitech-ai/alpha-desk`  
**Related repository:** `rsitech-ai/hlscreen`  
**Relationship to existing design:** Additive coverage expansion to `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md`; it does not replace the approved deterministic architecture.  
**Operating mode:** Read-only intelligence. No signing keys, private keys, order placement, copy-trading execution, or `/exchange` writes.

---

## 1. Executive decision

### Verdict: ACCEPT WITH CHANGES

The business objective is correct: Alpha Desk should become the authoritative, complete, evidence-preserving Hyperliquid intelligence system for HyperCore, HyperEVM, wallets, markets, orders, fills, liquidations, vaults, staking, borrow/lend, builders, and related derived intelligence.

The proposed generic Python/Kafka/Celery architecture should **not** replace the architecture already implemented in `alpha-desk`. The repository already has stronger foundations:

- immutable raw and canonical archives;
- deterministic canonical events and reducers;
- exact fixed-point accounting;
- provisional, committed, independent, reconciled, corrected, and expired evidence classes;
- RocksDB hot state, ClickHouse analytics, PostgreSQL control metadata, and NATS JetStream fan-out;
- point-in-time and bitemporal contracts;
- wallet, entity, market-intelligence, signal, replay, and execution-simulation libraries;
- extensive API contracts and replay/qualification tooling.

The correct program is therefore:

1. make source coverage explicit and machine-verifiable;
2. complete the official REST, WebSocket, node, historical-S3, and HyperEVM adapters;
3. map all state-affecting data into the existing canonical vocabulary;
4. represent snapshots and external-provider observations without pretending they are committed truth;
5. build the missing runtime wiring in `hl-core` and especially `hl-analytics`;
6. add wallet discovery, tiered tracking, history backfill, reconciliation, projections, alerts, and APIs;
7. port only the useful, non-duplicative parts of `hlscreen`;
8. continuously detect Hyperliquid API/schema drift so “all” remains a maintained property rather than a one-time claim.

### The most important architectural correction

“Track all public Hyperliquid data” must **not** mean “poll every wallet.”

The global activity foundation should be:

- committed node transaction blocks;
- node trades with both participants and starting positions;
- order-status streams;
- raw L4 book diffs and periodic state;
- miscellaneous ledger, staking, funding, and protocol events;
- official historical node/S3 datasets;
- raw HyperEVM blocks and receipts.

Wallet-specific REST and WebSocket endpoints then enrich, reconcile, and serve priority accounts. This avoids severe rate-limit fragility, survivorship bias, incomplete histories, and a false belief that a leaderboard seed list represents the whole venue.

---

## 2. Repository audit and current reality

### 2.1 Alpha Desk is the authority

`alpha-desk` should remain the canonical product and intelligence authority. Its approved architecture already defines a Rust 2024 modular monorepo with five deployables:

- `hl-capture`
- `hl-core`
- `hl-analytics`
- `hl-research`
- `hl-api`

The workspace already contains the important pure-domain crates:

- `hl-protocol`
- `canonical-events`
- `canonical-ledger`
- `canonical-state-store`
- `canonical-archive`
- `orderbook`
- `margin-models`
- `wallet-intelligence`
- `entity-graph`
- `feature-core`
- `market-intelligence`
- `signal-core`
- `execution-sim`
- `replay-engine`
- `storage-ports`
- `api-contracts`
- `telemetry`

The approved design’s core invariants remain binding:

- archive before fan-out;
- exact checked fixed-point values;
- deterministic synchronous reducers;
- one parser/reducer/feature path for live and replay;
- stable event identity and idempotency;
- point-in-time correctness;
- raw evidence preservation;
- ClickHouse is rebuildable and is never the only copy;
- red data health suppresses affected alpha;
- read-only V1.

### 2.2 What already exists and must not be rebuilt

The existing canonical event vocabulary already covers much of HyperCore:

- order accepted, rested, modified, partially filled, filled, cancelled, and rejected;
- trigger activation and TWAP lifecycle;
- matched trades with both counterparties, order IDs, TWAP IDs, client IDs, and starting positions;
- deposits, withdrawals, spot/perp/subaccount transfers;
- vault deposits and withdrawals;
- fees, builder fees, funding, and referral rewards;
- account, margin-mode, and leverage changes;
- liquidation start/fill/backstop and position settlement;
- market lifecycle, OI caps, margin tables, oracle/funding/context updates;
- DEX and outcome lifecycle.

The intelligence libraries already include:

- cashflow and equity reconstruction;
- wallet performance, style, intent, skill, maker/taker behavior, hedge detection, holding time, change points, whale classification, copyability, capacity, and slippage;
- temporal entity membership, counterparty graphs, leader/follower relationships, independence weighting, and originator diagnostics;
- smart-flow aggregation, crowding, entry maps, conviction, cross-asset state, sentiment, regime, market memory, and multi-step liquidation fragility;
- research holdouts and execution-simulation safeguards.

These are not backlog concepts. They are code that must be fed by complete, production-qualified evidence.

### 2.3 The actual gaps

The primary gaps are:

1. **Alpha Desk capture adapters:** `hl-capture` currently has node-file and node-stream adapters, but not a complete official `/info`, public WebSocket, S3, HyperEVM, and provider adapter suite.
2. **Protocol completeness:** the canonical vocabulary is broad, but newer protocol areas and miscellaneous ledger/staking/vault/borrow-lend variants need typed mappings and reducers.
3. **Runtime assembly:** `hl-capture` and `hl-api` are substantial, while `hl-core` has limited runtime assembly and `hl-analytics` is essentially a stub deployable.
4. **Wallet registry and discovery:** the intelligence math exists, but the durable discovery, prioritization, backfill, coverage, and scheduling control plane is missing.
5. **HyperEVM:** raw block/receipt ingestion, EVM projections, ABI/label registry, system-transaction decoding, and cross-layer reconciliation are not yet first-class.
6. **Analytical fact tables:** ClickHouse currently emphasizes feature, entity, market-memory, and signal tables rather than a complete normalized fact layer for every public domain.
7. **Coverage governance:** there is no single machine-readable inventory proving that every documented endpoint, subscription, node dataset, archive dataset, and schema variant is owned by an adapter, fixture, parser, and test.
8. **Product surfacing:** APIs are large, but the complete wallet/order/liquidation/vault/staking/EVM query surface and evidence-rich alerts still need implementation.

### 2.4 hlscreen’s role

`hlscreen` is a proven public spot market-data terminal and recorder. Its strongest reusable assets are:

- official REST and WebSocket parsing;
- connection lifecycle, reconnect, ping, inactivity, and subscription management;
- spot metadata and asset-context handling;
- trades, BBO, L2, mids, active context, and candle ingestion;
- feature formulas, microstructure, resilience, tradeability, composite confidence, and alert playbooks;
- recorder, replay, raw/normalized/Parquet paths, backfill, parity, benchmark, and qualification patterns;
- operational TUI concepts.

It must **not** become a second production source of truth inside Alpha Desk. Do not copy its SQLite run registry into the canonical system, do not run two independent production recorders for the same feed, and do not merge the repositories wholesale.

The integration rule is:

> Port reusable, neutral code and golden fixtures into Alpha Desk’s existing boundaries; keep one production capture authority.

Longer term, `hlscreen` can support an `alpha-desk` local API source mode, preserving it as a lightweight standalone terminal while avoiding duplicate production ingestion.

---

## 3. Scope and non-goals

### 3.1 In scope

The system shall collect, preserve, normalize, reconcile, process, and serve all publicly accessible data discoverable through:

1. Hyperliquid non-validating node outputs.
2. Official Hyperliquid `/info` REST.
3. Official Hyperliquid public WebSocket.
4. Official Hyperliquid historical S3 datasets.
5. Local and S3 HyperEVM block/receipt data.
6. Official HyperEVM JSON-RPC and system-transaction methods.
7. Public HyperCore read precompiles as reconciliation evidence.
8. Optional managed/indexed providers.
9. Optional community dashboards and explorers as discovery-only sources.
10. Operator-created labels, watchlists, and rule definitions.

It shall produce:

- complete replayable raw evidence;
- deterministic canonical HyperCore event history;
- exact reconstructed account, position, order, market, and order-book state;
- canonical HyperEVM blocks, transactions, receipts, logs, transfers, and known protocol actions;
- wallet discovery and dynamic tracking;
- point-in-time wallet/entity/market intelligence;
- rankings with coverage and confidence;
- evidence-backed alerts;
- REST/OpenAPI and resumable streaming APIs;
- native and web dashboard views;
- data-health and source-coverage surfaces.

### 3.2 Explicit non-goals

- No `/exchange` usage.
- No action signing.
- No storage of seed phrases, private keys, API wallets, or trading credentials.
- No automatic order placement, copy trading, liquidation participation, or vault actions.
- No evasion of official rate limits through anonymous proxy rotation.
- No claim that a third-party label, leaderboard, or wallet cluster is protocol truth.
- No claim of complete historical coverage when the source itself is truncated or has gaps.
- No GraphQL-first rewrite of the existing OpenAPI service.
- No immediate Kafka migration. NATS JetStream remains the operational fan-out system unless measured SLO evidence proves it insufficient.
- No Python runtime as the canonical production path. Python may remain useful for notebooks, provider exploration, and offline research.

---

## 4. Definition of “all”

“All” must be operationally testable.

### 4.1 Coverage definition

For a given network and build, the platform is “coverage-complete” when every enabled capability in the versioned capability manifest has:

- a named owner module;
- a raw observation contract;
- at least one golden fixture;
- a parser or explicit opaque-preservation policy;
- a trust/confirmation classification;
- a retention policy;
- a mapping target: canonical event, reconciled snapshot, reference snapshot, EVM fact, or discovery-only observation;
- a health metric and freshness target;
- an integration or replay test;
- a documented limitation;
- a current probe result.

Coverage is reported per domain and source, never as a single misleading boolean.

### 4.2 Capability states

Each capability has one of:

- `implemented`
- `implemented_unqualified`
- `qualified_live`
- `qualified_replay`
- `degraded`
- `unsupported_by_network`
- `source_unavailable`
- `schema_unknown`
- `disabled_by_policy`

### 4.3 Completeness dimensions

Every API response and dashboard aggregate must distinguish:

- **event completeness:** whether committed transitions are continuous;
- **state completeness:** whether current state reconciles;
- **wallet coverage:** how much of venue OI, volume, fills, or active accounts is represented;
- **historical depth:** earliest reliable point and known gaps;
- **source diversity:** primary, independent, reconciled, provider;
- **schema confidence:** known typed payload versus quarantined/opaque;
- **freshness:** event, knowledge, received, and publication lag.

---

## 5. Complete public data-domain inventory

The inventory below is the target domain model, not a recommendation to flatten everything into one table.

| Domain | Public evidence | Primary truth role | Target representation |
|---|---|---|---|
| Chain continuity | L1 blocks, heights, times, transaction blocks, state snapshots | Committed | block envelopes, checkpoints, gap records |
| Perp registry | validator perps, HIP-3 DEXs, assets, margin tables, leverage, deployers, oracle updaters, fee recipients, limits/status | Committed + reference snapshots | versioned market and DEX registry |
| Spot registry | tokens, pairs, token IDs, spot IDs, decimals, canonical/EVM mappings, deploy state, token details | Committed + reference snapshots | versioned token/pair registry |
| Outcome markets | outcome metadata, side encodings, lifecycle, resolution | Capability-gated | outcome registry and events |
| Quote assets | collateral/quote relationships and aligned-quote state | Reference + reconciled | quote-asset state |
| Market prices | trades, mids, BBO, mark, oracle, premium, impact prices | Committed trades + provisional/reconciled state | facts and snapshots |
| Order books | L2 snapshots, full L4 snapshots, raw per-order diffs | Committed for L4 where node-derived | deterministic L4 and derived L2 |
| Orders | accept, rest, modify, cancel, reject, trigger, TP/SL, status reasons | Committed | order state machine |
| TWAP | parent state, activation, history, slices, fills, termination/completion | Committed + user stream reconciliation | TWAP state machine |
| Fills/trades | both participants, maker/taker, IDs, fees, closed PnL, starting positions | Committed | trade/fill facts and position episodes |
| Candles | official snapshots/stream and self-built candles | Reconciled/derived | raw official candles plus deterministic derived candles |
| Funding | rates, history, predicted funding, user funding, distribution | Committed payments + snapshots | funding facts and rate snapshots |
| Open interest | market OI, caps, streaming caps, tracked-wallet OI | Reconciled + derived | market snapshots and coverage-aware aggregates |
| Account state | margin summaries, account value, withdrawable, modes, abstractions | Reconciled snapshots + committed transitions | account state and history |
| Positions | size, entry, leverage, liquidation, margin, unrealized/realized PnL | Reconstructed + reconciled | position state and episodes |
| Spot balances | total, hold, entry notional, transfers | Reconstructed + reconciled | balance ledger and snapshots |
| Portfolio history | account-value/PnL/volume windows | Provider/API observation | comparison input, not canonical ledger |
| Ledger | deposits, withdrawals, funding, internal, spot, perp, subaccount, class, vault, reward, liquidation updates | Committed where node-derived | exact double-entry-like ledger facts |
| Fees | maker/taker, spot/perp rates, rebates, builder fees, tiers, staking/referral discounts | Committed charges + reference snapshots | fee facts and schedule versions |
| Referrals | referrer relations, discount/reward state, claims | Committed + reconciled | relation edges and reward ledger |
| Builders/agents | approved builders, max builder fee, agents, extra agents, multisig, roles | Reconciled/reference + committed changes where visible | temporal authorization/relationship graph |
| Liquidations | user liquidations, liquidator/backstop, cancelled orders, cascade effects | Committed | liquidation lifecycle and stress facts |
| Vaults | metadata, leader, follower equities, deposits, withdrawals, distributions, commissions, performance | Committed + reconciled | vault registry, ledger, and performance |
| Staking | staking balances, delegations, withdrawals, rewards, validators, commission, jail state | Committed + reconciled | validator/delegator facts and state |
| Borrow/lend | user state, reserves, supply/withdraw/borrow/repay/liquidation and reserve configuration | Committed where node/CoreWriter-derived + reconciled | reserve/user state and actions |
| Market lifecycle | create, configure, halt, resume, delist, settle, cap and margin changes | Committed | versioned lifecycle events |
| HyperEVM chain | blocks, transactions, receipts, logs, system transactions, gas/burns | Committed | EVM canonical facts |
| HyperEVM assets | native/ERC20/ERC721/ERC1155 transfers, approvals, mints/burns | Committed derived from logs | typed EVM transfer facts |
| HyperEVM contracts | deployment, bytecode hashes, proxies, ABI/labels, protocol events | Committed facts + curated metadata | contract registry and decoded events |
| Core/EVM linkage | native and token transfers, system transactions, CoreWriter actions, precompile observations | Committed + reconciled | cross-layer linkage graph |
| External attribution | smart-money labels, exchange labels, leaderboard labels | Observation only | temporal attributed labels with provider provenance |
| Cross-venue | prices, funding, OI, basis, liquidity from other DEX/CEX feeds | Independent external | comparison features, not Hyperliquid truth |
| Data health | gaps, divergence, stale sources, unknown variants, coverage | System truth | findings, incidents, source health |
| Alerts | rule versions, evidence, delivery, acknowledgement, retraction | Derived | deterministic alert lifecycle |

---

## 6. Source hierarchy and trust model

### 6.1 Priority order

1. **Own local Hyperliquid non-validating node**
   - primary committed HyperCore source;
   - transaction blocks, trades, order statuses, raw book diffs, periodic state, miscellaneous events;
   - no HTTP polling bottleneck for venue-wide activity.

2. **Independent committed source**
   - second node, official node-data S3, or qualified managed node feed;
   - confirms continuity and detects local corruption/divergence.

3. **Official Hyperliquid REST and WebSocket**
   - WebSocket is the lowest-latency provisional state/event lane;
   - REST is snapshot, reconciliation, metadata, and bounded-history input;
   - neither silently overrides committed state.

4. **Official historical S3**
   - fills, transaction data, L2 snapshots, asset contexts, and raw EVM blocks;
   - useful for bootstrap/backfill, but each dataset’s documented gaps and timing limitations must be preserved.

5. **Managed providers**
   - QuickNode, GoldRush, Allium, Nansen, and similar;
   - scale, history, attribution, traces, batching, wildcard streams, and cross-checking;
   - every field carries provider, plan, dataset, extraction time, license, and coverage semantics.

6. **Community sources**
   - explorers, Dune dashboards, Hypurrscan-like services;
   - discovery and analyst hints only;
   - never silently promoted to canonical truth.

### 6.2 Evidence classes

Preserve the existing confirmation classes. Add a separate source-role descriptor:

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
```

A provider response can be reliable and still be `AttributionEnrichment`; reliability and authority are different concepts.

### 6.3 Record classes

The pipeline must keep five distinct classes:

1. **SourceObservationEnvelope**
   - byte-preserving source evidence;
   - all inputs enter here first.

2. **CanonicalEventEnvelope**
   - deterministic state-affecting HyperCore transitions;
   - stable event ID, block/transaction/event coordinates, typed payload.

3. **ReconciledSnapshotEnvelope**
   - point-in-time account, market, book, reserve, vault, or provider state;
   - used to bootstrap, compare, and repair;
   - does not create synthetic ledger transitions without an explicit correction event.

4. **ReferenceSnapshotEnvelope**
   - versioned metadata, schedules, margin tables, token registry, ABIs, labels.

5. **DerivedFactEnvelope**
   - features, rankings, clusters, signals, alerts, and outcomes;
   - always points to exact evidence and build provenance.

For HyperEVM, preserve canonical blocks/transactions/receipts/logs as chain facts. Decode known contract/system actions into typed derived protocol facts. Do not create one canonical enum variant for every arbitrary contract ABI event.

---

## 7. Target architecture

```text
                           ┌──────────────────────────────┐
                           │ Hyperliquid non-validating   │
                           │ node outputs                 │
                           │ replica_cmds / trades /      │
                           │ orders / L4 / misc / state   │
                           └──────────────┬───────────────┘
                                          │ committed
 ┌─────────────────────┐                  ▼
 │ Official WS          │        ┌───────────────────────┐       ┌────────────────────┐
 │ Official /info       │───────▶│       hl-capture      │──────▶│ Immutable raw       │
 │ Official S3          │        │ source adapters,      │ first │ observations        │
 │ HyperEVM local/S3    │        │ budgets, checkpoints, │       │ Parquet/object store│
 │ Optional providers   │        │ quarantine, failover  │       └──────────┬─────────┘
 └─────────────────────┘        └───────────┬───────────┘                  │
                                            │ archive acknowledgement       │
                                            ▼                               ▼
                                  ┌───────────────────────┐       ┌────────────────────┐
                                  │ NATS JetStream        │       │ canonical archive   │
                                  │ operational fan-out   │       │ + EVM fact archive  │
                                  └───────────┬───────────┘       └──────────┬─────────┘
                                              │                              │
                                              ▼                              │ replay
                                  ┌───────────────────────┐                  │
                                  │        hl-core        │◀─────────────────┘
                                  │ canonicalization,     │
                                  │ reducers, reconciliation,
                                  │ RocksDB exact hot state│
                                  └───────────┬───────────┘
                                              │ state deltas / facts
                                              ▼
                                  ┌───────────────────────┐
                                  │      hl-analytics     │
                                  │ projections, wallet   │
                                  │ registry/discovery,   │
                                  │ features, rankings,   │
                                  │ alerts, data health   │
                                  └───────┬────────┬──────┘
                                          │        │
                            ┌─────────────▼─┐  ┌──▼────────────────┐
                            │ ClickHouse    │  │ PostgreSQL         │
                            │ facts/history │  │ registry/control   │
                            └───────────────┘  └─────────┬─────────┘
                                                        │
                                                        ▼
                                             ┌─────────────────────┐
                                             │       hl-api        │
                                             │ OpenAPI, streams,   │
                                             │ evidence, health    │
                                             └──────────┬──────────┘
                                                        ▼
                                             Native desk / web / CLI
```

### 7.1 Keep the modular monolith

Do not immediately split wallet discovery, alerting, EVM ingestion, rankings, and reconciliation into separate deployables. Initially:

- source adapters remain modes inside `hl-capture`;
- canonical state and reconciliation remain inside `hl-core`;
- projections, wallet scheduling, derived intelligence, and alerts remain inside `hl-analytics`;
- serving remains inside `hl-api`.

Split a component only after measured independent scaling, failure-isolation, or deployment cadence requires it.

### 7.2 Storage roles remain unchanged

- **Immutable Parquet/object archive:** raw observations and canonical history.
- **RocksDB:** exact, low-latency canonical state and checkpoints.
- **ClickHouse:** analytical facts, histories, projections, features, rankings, and alert history.
- **PostgreSQL:** source catalog, wallet registry, labels, rules, jobs, providers, contracts, and control metadata.
- **NATS JetStream:** post-archive operational fan-out, not primary truth.
- **Redis:** not required. Add only as an explicitly disposable UI cache after profiling.

---

## 8. hlscreen integration without duplication

| hlscreen area | Action | Alpha Desk destination | Do not copy |
|---|---|---|---|
| `hls-hyperliquid` REST/WS parser | port and generalize with golden differential tests | `crates/hl-protocol/src/info`, `crates/hl-protocol/src/ws` | standalone canonical types |
| connection lifecycle | port reconnect, ping, inactivity, ack/snapshot handling | `services/hl-capture/src/adapters/public_ws.rs` | a second connection supervisor |
| subscription planner | port ideas; expand to all current subscription families and DEXes | `services/hl-capture/src/subscription_plan.rs` | hard-coded spot-only match statements |
| feature formulas | port formulas after exact numeric/PIT review | `feature-core`, `market-intelligence` | duplicate feature names with different semantics |
| resilience/tradeability | port as versioned features | `market-intelligence` | hidden f64 accounting paths |
| composite confidence | merge into existing health/confidence contracts | `feature-core`, `signal-core` | a second confidence taxonomy |
| raw/normalized recorder | reuse fixtures, failure tests, benchmarks | `canonical-archive`, `hl-capture` | SQLite as canonical truth |
| replay/parity | reuse qualification design and datasets | `replay-engine`, `tools`, `justfile` | independent replay semantics |
| backfill | reuse range and gap patterns | `hl-capture` historical jobs | separate run registry |
| TUI | reuse operational workflows and information hierarchy | optional Alpha Desk ops CLI/TUI or dashboard | duplicate product authority |
| screen DSL/presets | adapt to watchlist/query/rule definitions | PostgreSQL controls and `hl-api` | separate alert engine |
| WASM annotations | evaluate later as analyst-local extension | plugin boundary after core coverage | production alpha without signed schema/health |

A mandatory differential test suite shall feed identical `hlscreen` fixtures to both parsers/features and document every intentional difference. Ported code is considered complete only after parity or an approved semantic-difference record.

---

## 9. Capability manifest and drift control

### 9.1 New files

```text
config/hyperliquid/capabilities.toml
schemas/hyperliquid/capability-manifest-v1.schema.json
docs/hyperliquid/coverage-matrix.md                 # generated
fixtures/hyperliquid/<source>/<capability>/<case>.*
tools/hyperliquid-capabilities/
```

### 9.2 Manifest record

```toml
[[capability]]
id = "official.info.borrow_lend_user_state"
network = ["mainnet", "testnet"]
transport = "rest_info"
request_type = "borrowLendUserState"
domain = "borrow_lend"
source_role = "reconciliation_snapshot"
base_weight = 20
pagination = "none"
parser = "hl_protocol::info::borrow_lend::BorrowLendUserState"
fixture_set = "borrow_lend_user_state_v1"
retention = "raw_indefinite"
freshness_target_ms = 30000
status = "implemented_unqualified"
owner = "hl-capture"
```

Required fields:

- capability ID;
- network availability;
- source/transport;
- endpoint, request type, subscription type, node dataset, or S3 path;
- domain;
- source role and confirmation class;
- base and variable request weight;
- pagination semantics and limits;
- snapshot/incremental semantics;
- parser/type path;
- fixture set;
- state/event/fact target;
- freshness and continuity SLO;
- raw/canonical retention;
- owner;
- status;
- known limitations;
- last live probe and parser build.

### 9.3 CI and runtime checks

Add:

- `just hyperliquid-coverage-check`
- offline one-to-one checks between manifest entries, parser registry, fixture registry, and generated documentation;
- compatibility checks for protobuf field/tag changes;
- fixture hash stability;
- runtime unknown-field and unknown-variant counters;
- quarantine of unknown state-affecting variants;
- a scheduled external probe that generates a signed capability report;
- alerts when a documented capability starts failing, disappears, changes schema, or becomes available on another network.

The external probe must not make code generation depend on live internet access. CI remains deterministic and offline-capable.

---

## 10. Official REST `/info` ingestion

### 10.1 Module layout

```text
crates/hl-protocol/src/info/
  mod.rs
  request.rs
  response.rs
  registry.rs
  pagination.rs
  general.rs
  perpetuals.rs
  spot.rs
  accounts.rs
  orders.rs
  twap.rs
  funding.rs
  vaults.rs
  staking.rs
  fees_referrals.rs
  builders_agents.rs
  borrow_lend.rs
  outcomes.rs

services/hl-capture/src/adapters/
  info_rest.rs

services/hl-capture/src/
  request_budget.rs
  info_scheduler.rs
  backfill_scheduler.rs
```

### 10.2 Endpoint families

The manifest shall cover every documented read endpoint, including current general and newer areas:

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
- `userFunding`
- `userTwapSliceFills` and time/history variants when exposed
- `userVaultEquities`
- `vaultDetails`
- `userRole`
- `userRateLimit`
- `userFees`
- `referral`
- `subAccounts`
- multisig signers
- extra agents
- approved builders and maximum builder-fee approvals
- account and DEX abstraction state
- aligned quote token state
- borrow/lend user and reserve state
- delegator summary, delegations, history, rewards, and validator statistics
- all perpetual-specific metadata/context/funding/OI/deployment/status/annotation endpoints
- all spot metadata/context/state/deployment/token-detail endpoints
- outcome metadata when available
- newly added endpoint types discovered after this date.

### 10.3 Parsing policy

- Store the exact response bytes before parsing.
- Parse decimal strings into checked fixed-point domain types, never `f64` in canonical/accounting paths.
- Preserve unknown JSON fields in raw evidence.
- If an unknown field is non-state-affecting, continue with a warning and schema fingerprint change.
- If an unknown enum/variant can affect state, quarantine the typed mapping and degrade the capability.
- Keep network-specific asset-ID/name mappings versioned.
- Treat UI remappings as presentation metadata; canonical IDs use protocol names and IDs.
- Query actual master/subaccount addresses, never assume an agent address owns the account state.

### 10.4 Pagination

Time-range endpoints return bounded pages. Implement a cursor containing:

```rust
pub struct TimePageCursor {
    pub start_time_millis: i64,
    pub last_time_millis: Option<i64>,
    pub last_stable_id: Option<String>,
    pub overlap_millis: i64,
}
```

Rules:

1. use an overlap window;
2. deduplicate by stable protocol ID where available, otherwise a deterministic content identity;
3. handle multiple records with identical timestamps;
4. never advance solely to `last_timestamp + 1` when that could skip same-millisecond records;
5. detect repeated full pages that make no cursor progress;
6. record source truncation, earliest reliable time, and known gaps;
7. preserve `aggregateByTime` as a source parameter because aggregated and unaggregated fills are different observations.

---

## 11. Official WebSocket ingestion

### 11.1 Current target subscription families

The subscription registry shall support all documented families, currently including:

- `allMids`
- `notification`
- `webData3`
- `twapStates`
- `clearinghouseState`
- `openOrders`
- `candle`
- `l2Book`
- `trades`
- `orderUpdates`
- `userEvents`
- `userFills`
- `userFundings`
- `userNonFundingLedgerUpdates`
- `activeAssetCtx`
- `activeAssetData`
- `userTwapSliceFills`
- `userTwapHistory`
- `bbo`
- `spotState`
- `allDexsClearinghouseState`
- `allDexsAssetCtxs`

The registry is data-driven. Do not encode the authoritative list only in a Rust `match`.

### 11.2 Snapshot semantics

Many user streams send an initial snapshot marked `isSnapshot: true`.

Each stream processor must distinguish:

- subscription acknowledgement;
- initial snapshot;
- incremental update;
- duplicate snapshot after reconnect;
- out-of-order or stale update;
- source disconnect/gap;
- resubscription boundary.

Snapshot records are reconciled observations. Incremental messages may be provisional events until matched to committed evidence.

### 11.3 Connection planner

Respect current official per-IP limits:

- 10 WebSocket connections;
- 30 new connections per minute;
- 1000 subscriptions;
- 10 unique users across user-specific subscriptions;
- 2000 outgoing messages per minute;
- 100 simultaneous in-flight posts.

The planner allocates:

- market-wide connections by DEX/domain;
- priority-user connection slots;
- reserved capacity for failover and urgent reconciliation;
- deterministic reconnect jitter;
- subscription hashes and resume state;
- health and staleness per subscription.

Do not promise thousands of official user WebSocket subscriptions. Thousands of wallets are supported through committed node evidence, REST scheduling, snapshots, provider firehoses, and history—not by violating the official ten-user WebSocket limit.

---

## 12. Rate-limit-aware scheduling

### 12.1 Budget model

Implement weighted token buckets per egress origin and provider.

Official REST currently uses:

- 1200 aggregate weight per minute per IP;
- weight 2 for selected low-cost calls such as `l2Book`, `allMids`, `clearinghouseState`, `orderStatus`, `spotClearinghouseState`, and `exchangeStatus`;
- weight 60 for `userRole`;
- weight 20 for most other documented `/info` calls;
- response-size-dependent extra weight for selected history endpoints;
- candle extra weight based on returned rows;
- separate Explorer and EVM RPC limits.

Operate with a configurable safety envelope, initially 70–80% of the documented ceiling. Do not schedule at theoretical maximum.

### 12.2 Priority classes

| Priority | Work | Policy |
|---|---|---|
| P0 | node continuity, committed archive, source health | never starved |
| P1 | critical market and account reconciliation, alert-confirmation queries | reserved budget |
| P2 | active high-value wallet snapshots and current order/position state | dynamic |
| P3 | ordinary tracked-wallet refresh and metadata | opportunistic |
| P4 | historical backfill | pause under pressure |
| P5 | provider enrichment/community discovery | lowest |

### 12.3 Dynamic wallet cadence

Cadence depends on:

- recent node-observed activity;
- open notional and leverage;
- liquidation proximity;
- active alert rules;
- account value and coverage contribution;
- wallet tier;
- source freshness;
- expected request cost;
- provider availability.

A dormant address does not consume the same budget as a highly leveraged active whale.

### 12.4 Egress policy

Multiple owned egress paths or managed-provider endpoints may be configured for resilience and scale, but each must have:

- a stable egress ID;
- explicit operator ownership;
- source terms and plan metadata;
- independent token bucket;
- no anonymous proxy rotation;
- no concealment of automated access;
- auditable routing decisions.

---

## 13. Committed node and historical ingestion

### 13.1 Required node datasets

Extend existing node adapters and mappings for:

- `replica_cmds` transaction blocks;
- periodic ABCI state snapshots;
- node trades;
- node fills where used;
- node order statuses;
- node raw book diffs;
- miscellaneous events;
- full L4 snapshots/checkpoints;
- every new node-output schema discovered by qualification.

Node trades are especially valuable because they include buyer and seller, starting positions, order IDs, TWAP IDs, and client IDs. This creates venue-wide wallet discovery and accurate episode reconstruction without per-wallet polling.

### 13.2 L4 order book

Build deterministic L4 books:

1. bootstrap from an L4 snapshot computed from periodic state;
2. apply raw book diffs and order statuses in committed order;
3. preserve user, order ID, client ID, side, price, original size, remaining size, trigger/TPSL metadata, and time priority;
4. derive L2 as a projection;
5. compare derived L2 against official `l2Book`;
6. quarantine divergence beyond configured tolerances;
7. checkpoint and replay hash the L4 state.

### 13.3 Historical S3

Support resumable requester-pays ingestion for:

- official L2 snapshots and asset contexts;
- `node_fills_by_block`;
- older node fills/trades formats;
- explorer blocks;
- `replica_cmds`;
- HyperEVM raw blocks and receipts.

Each object manifest records:

- bucket/key;
- ETag and content hash;
- source dataset/version;
- byte count;
- first/last block and event time;
- parser build;
- import time;
- gap status;
- requester-pays cost metadata when available.

The platform must expose official limitations: some archives are monthly, may be missing data, and do not contain every desired dataset.

### 13.4 Capacity

Official node documentation estimates approximately 100 GB of logs per day under default output settings. Treat this as a core design input.

Initial retention policy:

- raw hot local: 7–14 days;
- raw warm object storage: 90 days or operator-configured;
- compacted canonical evidence: indefinite;
- canonical state checkpoints: frequent hot plus long-lived milestones;
- ClickHouse raw high-volume book deltas: configurable TTL after canonical archive and aggregate materialization;
- unknown/quarantined data: retained until resolved.

Do not delete raw evidence merely because a ClickHouse projection exists.

---

## 14. Canonical model expansion

### 14.1 Compatibility strategy

Keep `CanonicalEventEnvelope` V1. Its `event_kind` string and typed payload envelope permit additive message definitions. Append protobuf fields/messages only; never reuse tags. Use semantic schema versions and the existing upcaster.

Target version: `1.1.0` unless implementation discovers a breaking semantic issue requiring V2.

### 14.2 Candidate additive event kinds

Add only when committed source evidence exists:

- `NonUserOrderCancelled`
- `InternalTransfer`
- `AccountClassTransfer`
- `VaultCreated`
- `VaultDistribution`
- `VaultLeaderCommissionPaid`
- `RewardClaimed`
- `SpotGenesisApplied`
- `StakingDeposit`
- `StakingDelegated`
- `StakingUndelegated`
- `StakingWithdrawalQueued`
- `StakingWithdrawalCompleted`
- `ValidatorRewardPaid`
- `ValidatorJailed`
- `ValidatorUnjailed`
- `BorrowLendSupplied`
- `BorrowLendWithdrawn`
- `BorrowLendBorrowed`
- `BorrowLendRepaid`
- `BorrowLendLiquidated`
- `ReserveConfigurationChanged`
- `CoreEvmTransfer`
- `EvmCoreSystemAction`
- protocol-specific market/deployment lifecycle variants that cannot be represented by existing kinds.

Do not manufacture transition events from two REST snapshots unless an explicit correction process records the inference and uncertainty.

### 14.3 Snapshot families

Add typed reconciled snapshots for:

- market/asset contexts;
- account clearinghouse state per DEX;
- all-DEX account state;
- spot state;
- open orders;
- TWAP state/history;
- vault details and equities;
- fee and referral state;
- staking/delegator/validator state;
- borrow/lend user and reserve state;
- builders, agents, multisig, role, abstraction;
- quote alignment;
- provider positions/leaderboards;
- EVM precompile observations.

### 14.4 Stable identity

Use protocol coordinates where possible:

- HyperCore: chain, block height, transaction index/hash, event index, order/trade IDs;
- source-only REST/WS: source ID, capability, account/market, source sequence or event time, stable protocol ID, content hash;
- HyperEVM: chain ID, block hash/number, transaction hash/index, log index;
- provider: provider, dataset, provider record ID, extraction/knowledge time, content hash.

Correction records point to superseded fact IDs; they never mutate archived history.

---

## 15. Wallet registry, discovery, and tracking

### 15.1 PostgreSQL control model

Add a durable wallet registry:

```text
wallet_registry
wallet_discovery_evidence
wallet_tracking_policy
wallet_source_coverage
wallet_label_version
wallet_relation_evidence
wallet_backfill_cursor
wallet_refresh_lease
wallet_watchlist_membership
```

Core wallet fields:

- address and network;
- account role: user, subaccount, agent, vault, contract, validator, builder, unknown;
- master/subaccount/agent/multisig/builder/vault relations;
- first and last seen;
- first and last committed activity;
- discovery reasons and evidence IDs;
- dynamic priority and score components;
- refresh tier and next due time;
- current open notional, account value, leverage/risk indicators;
- source and historical coverage;
- labels with effective/knowledge time;
- exclusion/retention policy.

### 15.2 Discovery sources

- both sides of every node trade;
- user fields in order statuses and raw L4 diffs;
- transaction actions and miscellaneous events;
- liquidation participants;
- vault leaders/followers;
- staking delegators/validators;
- subaccount, agent, multisig, builder, and referral relationships;
- Core/EVM transfers and EVM logs;
- managed-provider leaderboards and smart-money feeds;
- operator watchlists;
- community/explorer hints, marked discovery-only.

### 15.3 Tracking tiers

Example policy:

- **Tier 0:** active incident/alert wallets; official user WebSocket slot where possible; immediate committed monitoring.
- **Tier 1:** active whales, high leverage, top-confidence wallets; seconds-level current-state reconciliation.
- **Tier 2:** high-value or high-skill active wallets; minute-level.
- **Tier 3:** ordinary active discovered wallets; 5–30 minute refresh.
- **Tier 4:** dormant/history-only; event-triggered or hourly/daily.
- **Tier 5:** labels/discovery candidates pending qualification.

Tier membership is explainable and versioned.

### 15.4 Historical backfill

For a newly discovered wallet:

1. fetch role and relationships;
2. capture current perp/spot/all-DEX state;
3. capture open/frontend orders and TWAP state;
4. backfill fills, funding, non-funding ledger, historical orders, TWAP slices/history, portfolio observations, fees/referrals, vault/staking/borrow-lend state;
5. use local node/S3/provider history to extend beyond official truncation;
6. record earliest reliable times per dataset;
7. replay into position episodes and equity;
8. reconcile current state;
9. compute features only when minimum coverage gates pass.

Backfill jobs are resumable, leased, idempotent, and budget-aware.

---

## 16. Deterministic account, position, and PnL reconstruction

### 16.1 Position episodes

A position episode starts when size moves from zero to non-zero and ends when it returns to zero or flips sign. Track:

- open/add/reduce/close/flip events;
- average entry and realized PnL;
- fees and builder fees;
- funding;
- leverage and margin mode changes;
- liquidation risk and actual liquidation;
- markouts at configured horizons;
- concurrent hedge legs and entity exposure;
- source coverage and correction history.

### 16.2 PnL decomposition

Maintain exact components:

- realized trading PnL;
- unrealized PnL;
- funding paid/received;
- maker rebates;
- trading fees;
- builder fees;
- deposits/withdrawals and internal transfers;
- referral rewards;
- vault returns/distributions/commissions;
- staking rewards;
- borrow/lend interest and liquidation effects;
- other protocol rewards.

Do not infer investment performance from raw account-value change without cashflow adjustment.

### 16.3 Performance outputs

Produce:

- cashflow-adjusted equity curve;
- time-weighted return;
- money-weighted return where meaningful;
- realized/unrealized/net PnL;
- max drawdown and drawdown duration;
- profit factor;
- closed-episode win rate;
- payoff ratio;
- volatility, downside deviation, Sharpe/Sortino with explicit interval assumptions;
- turnover, holding time, maker/taker ratio;
- asset, DEX, regime, and horizon decomposition;
- coverage/confidence.

The official `portfolio` endpoint is a comparison observation, not the only source of performance truth.

---

## 17. HyperEVM subsystem

### 17.1 Source strategy

Primary:

- local `evm_block_and_receipts` written after committed blocks are verified.

Fallback/independent:

- requester-pays `hl-mainnet-evm-blocks` and testnet bucket.

Supplemental:

- official JSON-RPC for latest/current queries and system transactions;
- archive/traces from a qualified provider or local archive implementation.

The official RPC lacks WebSocket support and does not support historical state for many methods. Raw committed block data is therefore the durable foundation.

### 17.2 New protocol modules

```text
crates/hl-protocol/src/evm/
  mod.rs
  block.rs
  transaction.rs
  receipt.rs
  log.rs
  system_transaction.rs
  asset.rs
  core_link.rs
  precompile.rs

services/hl-capture/src/adapters/
  evm_local.rs
  evm_s3.rs
  evm_rpc.rs
```

A separate crate is justified only if Ethereum dependency weight or architecture rules make `hl-protocol` unsuitable. Start inside `hl-protocol` and evaluate after profiling.

### 17.3 Indexed facts

- blocks and parent continuity;
- fast/slow block classification when explicitly supported, otherwise `unknown`;
- transactions;
- receipts;
- logs;
- contract creation;
- system transactions originating from HyperCore;
- native HYPE transfers;
- ERC-20 transfers and approvals;
- ERC-721 and ERC-1155 events;
- mint/burn and supply changes;
- gas used, base fee, priority-fee burn, failed transactions;
- known protocol events from registered ABIs;
- optional traces and internal transfers;
- CoreWriter actions;
- Core↔EVM transfers;
- precompile calls observed in traces/receipts where available.

### 17.4 Contract and ABI registry

PostgreSQL stores:

- chain ID and address;
- first/last seen;
- creation transaction;
- bytecode and implementation hashes;
- proxy relationships;
- ABI version and source;
- protocol/project label;
- verification confidence;
- effective and knowledge times;
- decoder build;
- license/attribution.

Unknown logs remain queryable by topic/address and raw bytes.

### 17.5 Cross-layer linkage

Create links between:

- HyperCore block height and EVM block construction context;
- EVM system transaction and originating HyperCore action;
- Core/EVM native and token transfers;
- token IDs, spot pairs, aligned quotes, and EVM contracts;
- CoreWriter action and resulting HyperCore state transition;
- read-precompile observation and canonical HyperCore state at the EVM block.

Read precompiles are valuable reconciliation evidence because their values correspond to HyperCore state at EVM block construction. They do not replace full L1 replay.

---

## 18. State reconstruction and reconciliation

### 18.1 Reconciliation is explicit

Create `ReconciliationFinding` with:

- finding ID;
- domain and subject;
- expected state hash/value;
- observed state hash/value;
- primary and comparison evidence;
- protocol/event/knowledge time;
- severity;
- tolerance policy version;
- first/last observed;
- status: open, auto-repaired, accepted variance, corrected, escalated;
- resolution evidence.

### 18.2 Core invariants

Examples:

- order terminal states cannot reopen without a new order identity;
- fill quantities cannot exceed original quantities;
- L4 aggregation equals derived L2;
- official L2 is within allowed timing/tick aggregation differences;
- position size reconstructs from committed fills and settlements;
- cashflows reconcile to account balances within known protocol semantics;
- funding and fee ledgers have valid signs/assets;
- market IDs and asset decimals are valid for the effective registry version;
- open-interest and margin-cap changes use the correct DEX and market version;
- Core/EVM transfers conserve amount after unit conversion and explicit fees;
- EVM parent hashes and block numbers are continuous;
- every canonical event is idempotent by event ID;
- replay from archive yields identical state and feature hashes.

### 18.3 Repair policy

- Missing raw source: backfill.
- Missing parser: quarantine and deploy parser; replay.
- Snapshot divergence with continuous committed events: investigate semantics before correction.
- Proven committed omission: emit a corrected canonical envelope with evidence.
- Provider disagreement: lower provider health; never rewrite committed state from provider alone.
- Unknown state-affecting variant: fail closed for affected reducer/features.

---

## 19. Storage schemas

### 19.1 Raw object layout

```text
raw/
  network=<mainnet|testnet>/
  source=<source_id>/
  capability=<capability_id>/
  ingest_date=YYYY-MM-DD/
  hour=HH/
  segment=<uuid>.<format>.<compression>
```

The raw observation index stores content hash, byte length, source offset, event/knowledge/received time, block coordinates where known, parser status, and archive URI.

### 19.2 Parquet additions

Keep existing:

- `raw-observations-v1.json`
- `canonical-events-v1.json`
- `market-positioning-v1.json`

Add:

- `reconciled-snapshots-v1.json`
- `reference-snapshots-v1.json`
- `unknown-payloads-v1.json`
- `evm-blocks-v1.json`
- `evm-transactions-v1.json`
- `evm-receipts-v1.json`
- `evm-logs-v1.json`
- `reconciliation-findings-v1.json`

Avoid account-address partitioning, which creates pathological high-cardinality directories. Partition by network/domain/date/hour and sort/index by account or market inside files.

### 19.3 RocksDB column families

Extend the state-store contract with:

- `meta`
- `market_state`
- `l2_book`
- `l4_orders`
- `account_state`
- `balances`
- `positions`
- `orders`
- `twap`
- `vaults`
- `staking`
- `borrow_lend`
- `evm_heads`
- `reconciliation`
- `event_seen`
- `checkpoints`

Changes require state-schema versioning and replay migration tests.

### 19.4 PostgreSQL migrations

Current migrations end at `0004`. Add:

- `0005_source_catalog.sql`
- `0006_wallet_registry.sql`
- `0007_backfill_jobs_and_coverage.sql`
- `0008_contract_abi_and_provider_registry.sql`
- `0009_watchlists_and_alert_rules.sql`

Do not duplicate existing entity annotations or cohort definitions; reference or extend them.

### 19.5 ClickHouse migrations

Current analytical migrations end at `0008`. Add:

- `0009_core_market_facts.sql`
- `0010_order_account_ledger_facts.sql`
- `0011_wallet_performance_and_rankings.sql`
- `0012_vault_staking_borrow_lend.sql`
- `0013_hyperevm_facts.sql`
- `0014_reconciliation_alerts_and_coverage.sql`

Representative tables:

- `fact_trade`
- `fact_fill`
- `fact_order_transition`
- `fact_l4_book_delta`
- `snapshot_market_context`
- `fact_funding`
- `fact_ledger_update`
- `snapshot_account`
- `snapshot_position`
- `fact_position_episode`
- `fact_liquidation`
- `dim_market_version`
- `dim_wallet_version`
- `fact_wallet_metric`
- `fact_wallet_ranking`
- `fact_vault_event`
- `fact_staking_event`
- `snapshot_borrow_lend_reserve`
- `fact_evm_block`
- `fact_evm_transaction`
- `fact_evm_log`
- `fact_evm_transfer`
- `fact_reconciliation_finding`
- `fact_alert_lifecycle`
- `snapshot_source_coverage`

Store typed analytical columns plus evidence IDs and raw archive URIs/hashes. Do not duplicate every raw JSON blob into ClickHouse.

---

## 20. Derived intelligence

### 20.1 Wallet intelligence

Use the existing libraries and add production projections for:

- current and historical positions;
- cashflow-adjusted equity;
- PnL and ROI over 24h, 7d, 30d, 90d, and all reliable history;
- realized/unrealized/funding/fee decomposition;
- drawdown, profit factor, episode win rate, turnover;
- style and intent;
- maker/taker behavior;
- holding-time profile;
- hedge and cross-asset behavior;
- behavior change points;
- skill with shrinkage and minimum evidence;
- whale status;
- copyability, capacity, latency and market-impact penalties;
- entity-adjusted independence.

### 20.2 Leaderboards

Never rank purely by raw PnL.

Leaderboard outputs include:

- raw net PnL;
- cashflow-adjusted return;
- account value/capital-at-risk;
- drawdown;
- sample size and active days;
- fees/funding;
- liquidity/capacity;
- copyability;
- entity independence;
- source coverage and confidence;
- earliest reliable history;
- whether result depends on provider-only evidence.

Default filters exclude:

- insufficient history;
- unadjusted external cashflows;
- unreconciled current state;
- known provider truncation;
- high entity duplication;
- obvious one-off lottery outcomes unless specifically requested.

### 20.3 Market intelligence

Build on existing smart-flow, crowding, regime, memory, and fragility modules:

- venue-wide and tracked-wallet aggression;
- long/short positioning with explicit coverage denominator;
- notional and OI concentration;
- whale opens/adds/reductions/closes/flips;
- liquidation proximity and cascade topology;
- funding/basis divergence;
- OI cap and margin-tier stress;
- order-book resilience and liquidity vacuums;
- entry-price maps and pain levels;
- cross-asset and cross-DEX relationships;
- oracle/mark/index anomalies;
- builder/deployer/vault concentration;
- HyperCore↔HyperEVM flow;
- staking and validator concentration;
- borrow/lend reserve utilization and liquidation risk;
- market lifecycle and delisting risk.

Do not call a tracked-wallet statistic “global” unless the committed venue-wide reconstruction supports that claim. Every aggregate carries:

```text
coverage_accounts
coverage_notional
coverage_oi_fraction
coverage_volume_fraction
coverage_source_set
coverage_confidence
```

### 20.4 Provider-derived intelligence

Nansen-style labels, smart-money classifications, or provider leaderboards are useful discovery priors. Store them as temporal provider observations. Alpha Desk’s own reconstructed metrics remain separate so analysts can compare:

- provider PnL versus canonical reconstruction;
- provider label versus observed behavior;
- provider position snapshot versus reconstructed/current official state.

---

## 21. Alert engine

### 21.1 Rule model

Rules are typed, versioned, deterministic, and point-in-time safe.

Example predicates:

- committed BTC short open/add above USD threshold;
- top-ranked independent wallet direction flip;
- position reaches configured liquidation-distance percentile;
- liquidation or backstop event;
- funding exceeds absolute/percentile threshold;
- OI cap or margin-table change;
- tracked cohort skew changes beyond threshold;
- vault drawdown or leader behavior change;
- borrow/lend reserve utilization/liquidation spike;
- validator jail/concentration change;
- large Core↔EVM transfer;
- source gap, stale state, or reconciliation divergence.

### 21.2 Lifecycle

```text
candidate -> provisional -> confirmed -> reconciled -> resolved
                                     \-> retracted
                                     \-> expired
```

Each alert has:

- rule and threshold version;
- deduplication key;
- subject and scope;
- detection/confirmation/reconciliation times;
- exact canonical event IDs and snapshot IDs;
- feature snapshot and build;
- data-health state;
- coverage;
- human-readable explanation generated from structured facts;
- delivery history and acknowledgement.

Language models may help summarize evidence, but they do not decide canonical direction, confirmation, or threshold crossing.

### 21.3 Delivery

- native/web dashboard;
- local notification;
- signed webhook;
- optional Telegram and Discord adapters in a separate egress worker;
- retry with idempotency;
- redaction policy;
- secrets isolated from capture/core processes.

---

## 22. API and streaming contracts

### 22.1 Response envelope

Every analytical response includes:

```json
{
  "schema_version": "1.x",
  "as_of": "protocol/event time",
  "knowledge_time": "what the system knew",
  "watermark": {"hypercore_height": 0, "evm_block": 0},
  "confirmation": "committed|reconciled|mixed",
  "source_coverage": {},
  "data_health": {},
  "build": {},
  "payload": {}
}
```

### 22.2 Market endpoints

- `GET /v1/markets`
- `GET /v1/markets/{market_id}`
- `GET /v1/markets/{market_id}/state`
- `GET /v1/markets/{market_id}/book/l2`
- `GET /v1/markets/{market_id}/book/l4`
- `GET /v1/markets/{market_id}/trades`
- `GET /v1/markets/{market_id}/funding`
- `GET /v1/markets/{market_id}/liquidations`
- `GET /v1/markets/{market_id}/crowding`
- `GET /v1/markets/{market_id}/fragility`
- `GET /v1/markets/{market_id}/flows`
- `GET /v1/dexes`
- `GET /v1/outcomes`

### 22.3 Wallet endpoints

- `GET /v1/wallets/{address}`
- `GET /v1/wallets/{address}/positions`
- `GET /v1/wallets/{address}/orders`
- `GET /v1/wallets/{address}/fills`
- `GET /v1/wallets/{address}/ledger`
- `GET /v1/wallets/{address}/performance`
- `GET /v1/wallets/{address}/equity`
- `GET /v1/wallets/{address}/style`
- `GET /v1/wallets/{address}/intent`
- `GET /v1/wallets/{address}/copyability`
- `GET /v1/wallets/{address}/relations`
- `GET /v1/wallets/{address}/coverage`
- `GET /v1/rankings/wallets`

### 22.4 Ecosystem endpoints

- `GET /v1/vaults`
- `GET /v1/vaults/{address}`
- `GET /v1/validators`
- `GET /v1/staking/delegators/{address}`
- `GET /v1/borrow-lend/reserves`
- `GET /v1/borrow-lend/users/{address}`
- `GET /v1/builders`
- `GET /v1/agents/{address}`
- `GET /v1/quotes/aligned`

### 22.5 HyperEVM endpoints

- `GET /v1/evm/blocks/{number_or_hash}`
- `GET /v1/evm/transactions/{hash}`
- `GET /v1/evm/addresses/{address}`
- `GET /v1/evm/contracts/{address}`
- `GET /v1/evm/transfers`
- `GET /v1/evm/protocol-events`
- `GET /v1/evm/core-links/{tx_or_event_id}`

### 22.6 Operations and evidence

- `GET /v1/data-health`
- `GET /v1/sources`
- `GET /v1/coverage`
- `GET /v1/reconciliation`
- `GET /v1/evidence/{id}`
- `GET /v1/alerts`
- `POST /v1/watchlists`
- `POST /v1/alert-rules`
- `POST /v1/alerts/{id}/acknowledge`

Control writes are local Alpha Desk metadata only.

### 22.7 Streaming

Use existing resumable stream contracts for:

- market state;
- trades and orders;
- wallet state;
- features;
- alert lifecycle;
- data health;
- source coverage.

Clients receive sequence, watermark, snapshot/resume token, and stale-state markers. A client reconnect never presents cached data as current without explicit freshness metadata.

---

## 23. Dashboard requirements

### 23.1 Market workspace

- real-time L2/L4 depth;
- trades and aggression;
- OI/funding/oracle/mark;
- crowding and tracked coverage;
- liquidation clusters and fragility paths;
- whale activity;
- cross-DEX comparisons;
- source health and watermark.

### 23.2 Wallet workspace

- account and relationship summary;
- current positions/orders/TWAPs;
- fill timeline;
- equity and PnL decomposition;
- drawdown and episode statistics;
- style, intent, skill, copyability, capacity;
- counterparties/entity links;
- evidence and coverage;
- alerts and change points.

### 23.3 Ecosystem workspace

- vault leaderboard and risk;
- validators/delegations/jail state/concentration;
- builders and fee activity;
- HIP-3 DEX/deployer status;
- aligned quote state;
- borrow/lend reserves and utilization;
- outcome markets;
- Core↔EVM flow.

### 23.4 Operations workspace

- source status and rate budgets;
- block/event lag;
- subscription count and user slots;
- backfill queues;
- raw archive throughput;
- unknown variants;
- reconciliation findings;
- storage growth;
- alert-delivery health.

---

## 24. Security, privacy, and legal controls

- Compile-time and runtime denylist for `/exchange` and signing dependencies in production capture/core builds.
- No private keys, seed phrases, API agents, or user credentials.
- Provider secrets only in the adapter/egress process that requires them.
- Network egress allowlist.
- TLS verification and certificate/host validation.
- Raw data and labels have retention/licensing metadata.
- Provider contracts record redistribution rights; API responses can suppress fields that cannot be redistributed.
- Public wallet data is still treated as potentially sensitive behavioral data: access controls, audit logs, and purpose limitation apply.
- Human-created allegations or identity labels require provenance, confidence, effective time, and reviewer metadata.
- Community labels never overwrite protocol roles.
- Rate-limit policies comply with source terms; no anonymous proxy evasion.
- Read-only release manifests are scanned to ensure no execution/signing capability is present.

---

## 25. Observability and service-level objectives

Initial SLO targets must be measured and revised from production evidence.

### 25.1 Latency

- provisional critical WS event publication: p99 < 500 ms after receipt;
- committed node event archived and published: p99 < 1.5 s after local file availability;
- canonical state publication: p99 < 2 s after committed evidence;
- critical alert confirmation: p99 < 3 s where source permits;
- standard wallet current-state refresh: per tracking tier;
- API p95: < 250 ms for hot state, < 2 s for bounded analytical queries.

### 25.2 Correctness

- zero silent discard of unknown state-affecting variants;
- replay produces identical canonical state hash;
- duplicate delivery and crash/restart produce no duplicate effects;
- every alert/ranking has evidence and data-health metadata;
- exact decimals round-trip through API;
- bitemporal/PIT leakage tests pass;
- all source gaps are recorded.

### 25.3 Capacity qualification

Test at least:

- all markets and DEXes;
- full L4 stream;
- ~100 GB/day raw node input design load;
- 10,000+ wallet registry entries;
- thousands of active wallet refresh jobs;
- multi-year backfill;
- HyperEVM block/log throughput;
- alert bursts during liquidation cascades;
- ClickHouse rebuild from archive.

### 25.4 Core metrics

- source lag, height, sequence, reconnects, gaps;
- request weight consumed/reserved/rejected;
- WebSocket connections/subscriptions/unique users;
- raw bytes and archive acknowledgement latency;
- parser successes/unknown fields/unknown variants/quarantine;
- canonical events per kind;
- reducer lag and state hash;
- reconciliation divergence;
- wallet coverage and refresh lateness;
- ClickHouse projection lag;
- alert candidate/confirm/retract/delivery;
- EVM head/continuity/log decoder health;
- storage growth and compaction;
- provider latency, disagreement, cost, and license state.

---

## 26. Testing strategy

### 26.1 Protocol and fixtures

- golden payload tests for every capability;
- mainnet and testnet fixtures;
- unknown field/enum/variant tests;
- decimal boundary and overflow tests;
- asset-ID/name mapping tests for validator perps, HIP-3, spot, and outcomes;
- official snapshot/incremental WebSocket tests;
- order-status reason coverage;
- pagination collision and truncation tests.

### 26.2 Differential hlscreen tests

- parser equivalence for shared REST/WS messages;
- lifecycle/reconnect behavior;
- feature formula parity;
- intentional difference records;
- qualification replay against recorded hlscreen runs.

### 26.3 Deterministic replay

- full event replay;
- duplicate delivery;
- out-of-order provisional evidence;
- committed correction;
- checkpoint restore;
- L4 snapshot plus diffs;
- wallet position episodes;
- account/ledger/PnL;
- vault/staking/borrow-lend;
- HyperEVM block/receipt/log replay;
- cross-layer transfer linkage.

### 26.4 Integration and chaos

- official API emulator with weighted limits and 429s;
- WebSocket drop/reconnect/snapshot duplication;
- node file truncation/rotation;
- S3 missing object and hash mismatch;
- NATS/PostgreSQL/ClickHouse outages;
- disk pressure and archive backlog;
- provider disagreement;
- EVM parent-hash gap;
- restart during archive/fan-out boundary;
- alert deduplication and delivery retry.

### 26.5 Research correctness

- point-in-time leakage;
- cashflow adjustment;
- survivor/discovery bias reports;
- provider-history truncation;
- entity duplication;
- ranking stability and shrinkage;
- copyability after latency, fees, spread, depth, and impact;
- feature suppression under red health.

### 26.6 Verification commands

Retain:

- `just verify`
- `just capture-e2e`
- `just capture-outage-e2e`
- `just capture-failover-e2e`
- `just capture-soak`
- existing state-replay commands.

Add:

- `just hyperliquid-coverage-check`
- `just public-api-fixtures`
- `just public-ws-replay`
- `just l4-replay-e2e`
- `just wallet-backfill-e2e`
- `just wallet-reconciliation-e2e`
- `just analytics-projection-e2e`
- `just hyperevm-replay-e2e`
- `just cross-layer-reconciliation-e2e`
- `just alert-lifecycle-e2e`
- `just full-coverage-soak`

---

## 27. Deployment topology

### 27.1 Initial production topology

- one primary non-validating node host;
- optional independent node or managed committed stream;
- `hl-capture` near the node storage;
- object/archive storage;
- `hl-core` with local fast RocksDB storage;
- `hl-analytics` close to ClickHouse/PostgreSQL;
- `hl-api` on the internal application network;
- NATS JetStream with durable storage and replication appropriate to deployment size;
- Prometheus/VictoriaMetrics and OpenTelemetry;
- separate optional provider/notification egress worker.

### 27.2 Failure behavior

- Node unavailable: provisional official WS continues; committed-dependent features become stale/degraded.
- Official API unavailable: committed capture continues; reconciliation and wallet snapshots degrade.
- Provider unavailable: provider-derived labels/history degrade only.
- ClickHouse unavailable: raw/canonical capture and hot state continue; projections backlog.
- PostgreSQL unavailable: existing schedules continue from leased cache where safe; new control writes stop.
- NATS unavailable: archive continues to bounded spool; fan-out resumes after recovery.
- Disk pressure: backfill and low-priority capture pause first; critical committed source fails closed before data loss.

---

## 28. Acceptance criteria

The expansion is complete only when:

1. The capability manifest covers every current official endpoint, subscription, node dataset, historical dataset, and HyperEVM source within scope.
2. Every enabled capability has a fixture, parser/opaque policy, health metric, owner, and replay/integration test.
3. Venue-wide committed trades, order statuses, L4 changes, and miscellaneous events are continuously archived.
4. Official REST/WS are live-qualified with documented budgets and snapshot semantics.
5. Wallets are discovered from committed activity, not only external leaderboards.
6. At least 10,000 wallet registry records can be scheduled without exceeding configured source budgets.
7. Current wallet positions/orders/balances reconcile within approved semantics.
8. Historical coverage and truncation are visible per wallet/dataset.
9. L4 replay is deterministic and derived L2 reconciles.
10. Vault, staking, builder, referral, aligned quote, borrow/lend, HIP-3, and outcome capabilities are represented or explicitly capability-gated.
11. HyperEVM blocks/transactions/receipts/logs and system transactions are archived and queryable.
12. Core/EVM transfers and known CoreWriter actions are linked.
13. ClickHouse fact projections can be rebuilt from immutable archives.
14. Existing wallet/entity/market intelligence is driven by production facts through `hl-analytics`.
15. Rankings are cashflow-adjusted, point-in-time safe, entity-aware, and coverage-labelled.
16. Alerts have deterministic lifecycle and evidence bundles.
17. `hlscreen` shared parsers/features pass differential tests and only one production capture authority is deployed.
18. No execution/signing code or secrets are present in release artifacts.
19. Load, soak, chaos, restore, and replay gates pass.
20. The dashboard exposes data health, source coverage, and limitations alongside intelligence.

---

## 29. Key risks and mitigations

| Risk | Consequence | Mitigation |
|---|---|---|
| API/schema changes | silent parser corruption | capability manifest, schema fingerprints, quarantine, live probes |
| node log volume | disk exhaustion | retention tiers, compaction, capacity alarms, object archive |
| false completeness | bad rankings/signals | coverage dimensions and earliest reliable time on every result |
| provider dependency | vendor lock-in or disagreement | adapters behind ports, raw evidence, provider provenance, own node truth |
| truncated wallet history | biased PnL/skill | node/S3/provider extension, explicit gaps, minimum coverage gates |
| duplicate hlscreen implementation | inconsistent truth | one capture authority, differential porting, Alpha API source mode |
| premature microservices | operational complexity | retain five deployables until measured need |
| REST over-polling | bans/gaps | node-first discovery, weighted scheduler, safety reserve |
| incorrect event inference from snapshots | double counting | separate snapshot class, explicit corrections only |
| EVM archive gaps | incomplete protocol analytics | local raw source + S3 fallback + qualified archive provider |
| bad wallet/entity labels | misleading attribution | temporal provenance, confidence, review, never overwrite protocol role |
| ranking overfit | copy-trading losses | shrinkage, locked definitions, capacity/slippage, research gates |
| stale UI | unsafe decisions | watermark, freshness, stale marker, resume protocol |
| licensing limits | unlawful redistribution | provider registry, field-level policy, internal-only defaults |

---

## 30. Design decisions to freeze

1. Alpha Desk remains the only canonical intelligence authority.
2. `hlscreen` is a source of reusable code/fixtures and an optional thin client, not a second production truth system.
3. Committed node evidence is the venue-wide foundation.
4. REST/WS are provisional/reconciliation lanes.
5. Providers are enrichment, scale, independent checks, or history—not silent canonical overrides.
6. NATS JetStream remains the operational bus; Kafka is not introduced without measured evidence.
7. Rust remains the production runtime; Python is optional research tooling.
8. Existing five deployables remain the initial deployment boundary.
9. Raw archive precedes publication.
10. Exact fixed-point and bitemporal semantics remain mandatory.
11. Snapshots do not synthesize ledger events by default.
12. HyperEVM raw blocks/receipts are first-class canonical chain facts.
13. “All” is defined by a maintained capability manifest and coverage report.
14. Read-only scope is non-negotiable for this expansion.

---

## 31. Recommended implementation order

1. Capability manifest and current coverage audit.
2. Shared official REST/WS protocol layer and hlscreen differential fixtures.
3. Weighted REST scheduler and complete public WebSocket lifecycle.
4. Node dataset completion and L4 reconstruction.
5. Historical S3 backfill.
6. Canonical event/snapshot additions and reducers.
7. `hl-core` runtime assembly.
8. Wallet registry, discovery, priorities, and history backfill.
9. HyperEVM local/S3/RPC ingestion and cross-layer links.
10. ClickHouse fact projections and `hl-analytics` runtime.
11. Wallet performance, rankings, market aggregates, and coverage.
12. Alert lifecycle.
13. API/dashboard expansion.
14. qualification, load, chaos, restore, and release gates.
15. hlscreen Alpha API source mode and removal of duplicate production ingestion.

The companion implementation plan defines PR-sized tasks, exact file paths, tests, and verification commands.

---

## Appendix A — Official limits and source facts used by this design

As of 2026-08-19:

- HyperCore orders, cancels, trades, and liquidations are public onchain activity with one-block finality.
- Official REST uses a shared 1200-weight/minute/IP budget with endpoint-specific weights and variable response costs.
- Official WebSocket limits include 10 connections, 1000 subscriptions, and 10 unique user addresses across user-specific subscriptions.
- Time-range `/info` responses are bounded; fills and historical orders have documented recent-history caps.
- Node output includes transaction blocks, periodic state, trades with both participants, order statuses, raw book diffs, miscellaneous events, and L4 snapshots.
- Default node logging can be approximately 100 GB/day.
- Official historical datasets have documented timing and completeness limitations.
- HyperEVM mainnet chain ID is 999 and testnet is 998.
- Official HyperEVM RPC has no WebSocket and limited historical-state support.
- Raw HyperEVM blocks/receipts are available locally and in requester-pays S3, MessagePack + LZ4.
- HyperEVM uses interleaved fast and slow blocks.
- HyperCore read precompiles expose positions, balances, vault, staking, oracle, and L1-height state at EVM block construction.
- Current `/info` documentation includes newer aligned-quote, borrow/lend, and approved-builder queries.
- Outcome asset encodings are distinct from spot and perp IDs.

---

## Appendix B — Current official capability bootstrap inventory

This list is the bootstrap inventory verified against the current official documentation and SDK surface on 2026-08-19. It is **not** the permanent source of truth: `config/hyperliquid/capabilities.yaml`, generated probes, schema fingerprints, and the coverage report must detect additions, removals, renamed fields, network-only capabilities, and changed semantics.

### Shared/general `/info` request types

- `allMids`
- `openOrders`
- `frontendOpenOrders`
- `userFills`
- `userFillsByTime`
- `recentTrades`
- `userRateLimit`
- `orderStatus`
- `l2Book`
- `candleSnapshot`
- `exchangeStatus`
- `historicalOrders`
- `userTwapSliceFills`
- `userTwapSliceFillsByTime`
- `twapHistory`
- `subAccounts`
- `userToMultiSigSigners`
- `portfolio`
- `referral`
- `userFees`
- `userRole`
- `userAbstraction`
- `userDexAbstraction`
- `extraAgents`
- `approvedBuilders`
- `userVaultEquities`
- `vaultDetails`
- `delegatorSummary`
- `delegations`
- `delegatorHistory`
- `delegatorRewards`
- `validatorStats`
- `alignedQuoteTokenInfo`
- `borrowLendUserState`
- `borrowLendReserveState`
- `allBorrowLendReserveStates`

### Perpetual-specific `/info` request types

- `perpDexs`
- `meta`
- `metaAndAssetCtxs`
- `allPerpMetas`
- `clearinghouseState`
- `userFunding`
- `userNonFundingLedgerUpdates`
- `nonUserFundingUpdates`
- `fundingHistory`
- `predictedFundings`
- `perpsAtOpenInterestCap`
- `perpDeployAuctionStatus`
- `activeAssetData`
- `perpDexLimits`
- `perpDexStatus`
- `perpAnnotation`
- `perpCategories`
- `perpConciseAnnotations`

### Spot and outcome `/info` request types

- `spotMeta`
- `spotMetaAndAssetCtxs`
- `spotClearinghouseState`
- `spotDeployState`
- `spotPairDeployAuctionStatus`
- `tokenDetails`
- `outcomeMeta` — capability/network-gated; currently documented as testnet-only

### Official WebSocket subscription types

1. `allMids`
2. `notification`
3. `webData3`
4. `twapStates`
5. `clearinghouseState`
6. `openOrders`
7. `candle`
8. `l2Book`
9. `trades`
10. `orderUpdates`
11. `userEvents`
12. `userFills`
13. `userFundings`
14. `userNonFundingLedgerUpdates`
15. `activeAssetCtx`
16. `activeAssetData`
17. `userTwapSliceFills`
18. `userTwapHistory`
19. `bbo`
20. `spotState`
21. `allDexsClearinghouseState`
22. `allDexsAssetCtxs`

The first message on streaming history feeds may be a snapshot and must be represented with `isSnapshot` semantics rather than replayed as a new event. The planner must enforce the documented global WebSocket connection, subscription, user-address, message, and in-flight limits.

### Capability-manifest rules for every entry

Each capability record must contain:

- network and source family;
- request/subscription shape and dimensions;
- current documentation URL and last verified time;
- expected source role: committed, provisional, reconciliation, enrichment, or evidence-only;
- rate weight and variable response-cost rule;
- history/window/truncation limits;
- snapshot/bootstrap semantics;
- parser/schema owner;
- raw archive partition;
- canonical event or snapshot mapping;
- fixture hash and schema fingerprint;
- health metric and freshness SLO;
- live/replay qualification status;
- known limitations and redistribution policy.

---

## Appendix C — Source references

### Alpha Desk repository

- `README.md`
- `Cargo.toml`
- `docs/STATUS.md`
- `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md`
- `docs/superpowers/plans/2026-07-24-00-hyperliquid-alpha-desk-program-roadmap.md`
- `crates/canonical-events/src/lib.rs`
- `schemas/proto/canonical/v1/events.proto`
- `crates/canonical-ledger/src/*`
- `crates/wallet-intelligence/src/lib.rs`
- `crates/entity-graph/src/lib.rs`
- `crates/market-intelligence/src/lib.rs`
- `services/hl-capture/src/*`
- `services/hl-capture/src/bus/subjects.rs`
- `services/hl-api/src/*`
- `schemas/postgres/*`
- `schemas/clickhouse/*`
- `schemas/parquet/*`
- `justfile`

### hlscreen repository

- `README.md`
- `Cargo.toml`
- `crates/hls-hyperliquid/src/*`
- `crates/hls-features/src/*`
- `crates/hls-store/src/*`

### Official Hyperliquid documentation

- https://hyperliquid.gitbook.io/hyperliquid-docs/
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/perpetuals
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint/spot
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/rate-limits-and-user-limits
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/asset-ids
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/nodes/l1-data-schemas
- https://hyperliquid.gitbook.io/hyperliquid-docs/historical-data
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/hyperevm
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/hyperevm/dual-block-architecture
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/hyperevm/raw-hyperevm-block-data
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/hyperevm/json-rpc
- https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/hyperevm/interacting-with-hypercore
- https://hyperliquid.gitbook.io/hyperliquid-docs/hypercore/staking
- https://hyperliquid.gitbook.io/hyperliquid-docs/hypercore/aligned-quote-assets
- https://hyperliquid.gitbook.io/hyperliquid-docs/trading/builder-codes

### Optional provider documentation

- https://www.quicknode.com/docs/hyperliquid
- https://docs.nansen.ai/api/hyperliquid
- https://goldrush.dev/docs/goldrush-hyperliquid/overview
- https://docs.allium.so/historical-data/supported-blockchains/hyperliquid/overview

Provider statements are treated as vendor claims until independently qualified.

---

## Appendix D. Requirement IDs

These IDs index the spec plus recorded parent adjudications. They do not add product behavior.

Approved base design: `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md`. Frozen V1 review tags: `design-approved-v1.0.0`, `spec-v1.0.0`.

Task mapping, target tests, and acceptance evidence live in `docs/superpowers/plans/2026-08-19-hyperliquid-full-coverage-traceability.md`.

| ID | Spec locus | Requirement |
| --- | --- | --- |
| HLCOV-SRC-001 | §1, §6.1, §30.3 | Committed node activity is the venue-wide foundation. Do not poll every wallet. |
| HLCOV-SRC-002 | §6.1 | An independent committed source confirms continuity and detects local corruption. |
| HLCOV-SRC-003 | §6.1, §30.4 | Official REST and WebSocket are provisional/reconciliation lanes and never silently override committed state. |
| HLCOV-SRC-004 | §6.1, §13.3 | Official historical S3 backfill preserves each dataset's documented gaps and timing limits. |
| HLCOV-SRC-005 | §6.1, §30.5 | Managed providers carry provenance and never silently overwrite canonical truth. Community sources are discovery-only. |
| HLCOV-SRC-006 | §6.2, §6.3 | Evidence carries `SourceRole`. The five record classes stay distinct. All inputs enter `SourceObservationEnvelope` first. |
| HLCOV-SRC-007 | §3.1 | In-scope sources are node outputs, `/info`, public WS, historical S3, HyperEVM, read precompiles, optional providers, community discovery, and operator labels/watchlists/rules. |
| HLCOV-SRC-008 | §10.3 | Store exact REST bytes before parse. Canonical/accounting paths use checked fixed-point, never `f64`. Unknown non-state fields warn. Unknown state-affecting variants quarantine. |
| HLCOV-SRC-009 | §10.4 | Time-range pagination uses overlap, stable-ID or content-hash dedup, same-millisecond records, truncation/gap recording, and `aggregateByTime` as a distinct observation. |
| HLCOV-SRC-010 | §12.1, §12.2, §12.3 | Weighted REST token buckets per egress, 70-80% safety envelope, P0-P5 priorities, and activity-based wallet cadence. |
| HLCOV-SRC-011 | §12.4, §3.2 | Named egress paths with independent budgets. No anonymous proxy rotation. |
| HLCOV-SRC-012 | §13.1 | Required node datasets include transaction blocks, periodic state, trades, fills, order statuses, raw book diffs, miscellaneous events, and L4 snapshots. |
| HLCOV-SRC-013 | §13.3 | Historical S3 ingestion is resumable requester-pays with per-object manifests. |
| HLCOV-SRC-014 | §13.4 | Design for ~100 GB/day node logs. Do not delete raw evidence because a ClickHouse projection exists. |
| HLCOV-PROTO-001 | §4.1, §30.13 | "All" is coverage-complete only when every enabled manifest capability has owner, fixture, parser or opaque policy, trust class, retention, mapping, health, test, limitation, and probe. |
| HLCOV-PROTO-002 | §4.2 | Capability status is one of the enumerated states in §4.2. |
| HLCOV-PROTO-003 | §4.3 | Every API response and dashboard aggregate distinguishes the completeness dimensions in §4.3. |
| HLCOV-PROTO-004 | §9.1 | Manifest is `config/hyperliquid/capabilities.toml` plus JSON schema and generated coverage matrix. |
| HLCOV-PROTO-005 | §9.2, App B | Every capability record includes the required fields in §9.2 and Appendix B. |
| HLCOV-PROTO-006 | §9.3 | `just hyperliquid-coverage-check` is offline and deterministic. Live probes must not be required for code generation. |
| HLCOV-PROTO-007 | §14.1 | `CanonicalEventEnvelope` stays V1. Target additive `1.1.0`. Append fields/messages only. Never reuse protobuf tags. No V2. |
| HLCOV-PROTO-008 | §14.2, §14.3, §30.11 | Additive event kinds require committed evidence. Typed reconciled snapshots are separate. Snapshots do not synthesize ledger events by default. |
| HLCOV-PROTO-009 | §14.4 | Stable identity uses protocol coordinates. Correction records never mutate archived history. |
| HLCOV-PROTO-010 | §8, §30.2, §28.17 | `hlscreen` is reusable code/fixtures and an optional thin client, not a second production truth system. Differential tests are mandatory once fixtures are in scope. |
| HLCOV-PROTO-011 | §5, §28.10 | Public domains including vault, staking, builder, referral, aligned quote, borrow/lend, HIP-3, and outcome are represented or explicitly capability-gated. |
| HLCOV-PROTO-012 | §10.2, §11.1, §11.2, §11.3 | Manifest covers documented `/info` families. WS registry is data-driven. Snapshot vs incremental vs reconnect classes are distinct. Planner enforces official WS limits. |
| HLCOV-CORE-001 | §7.1, §30.8 | Keep the five deployables. Do not split a deployable per domain until measured need. |
| HLCOV-CORE-002 | §7.2 | Storage roles stay Parquet/object archive, RocksDB, ClickHouse, PostgreSQL, NATS JetStream. Redis is not required. |
| HLCOV-CORE-003 | §3.2, §30.6 | NATS JetStream remains the operational bus. No Kafka without measured SLO evidence. |
| HLCOV-CORE-004 | §2.1, §30.9 | Raw evidence is archived before acknowledgement/publication. Live and replay share parser, mapping, reducer, feature, and signal paths. |
| HLCOV-CORE-005 | §13.2, §28.9 | L4 reconstruction is deterministic. Derived L2 reconciles against official `l2Book`. |
| HLCOV-CORE-006 | §16, §18.2 | Missing deterministic reducers and the listed reconstruction invariants. |
| HLCOV-CORE-007 | §18.1, §18.3 | Reconciliation is an explicit finding. Unknown state-affecting variants fail closed for affected reducers/features. Providers never rewrite committed state alone. |
| HLCOV-CORE-008 | §19.3 | RocksDB column-family extensions are versioned and covered by replay migration tests. |
| HLCOV-CORE-009 | §19.1, §19.2 | Raw object layout and Parquet additions in §19. Avoid account-address directory partitioning. |
| HLCOV-CORE-010 | §19.4 | PostgreSQL control schemas for source catalog, wallet registry, backfill/coverage, ABI/provider registry, and watchlists/alert rules. |
| HLCOV-CORE-011 | §7, §27.2 | Production `hl-core` runtime. Failure behavior in §27.2, including archive-without-NATS and stale committed features when the node is down. |
| HLCOV-WALLET-001 | §15.1 | Durable wallet registry and related control tables with the core fields in §15.1. |
| HLCOV-WALLET-002 | §15.2, §28.5 | Wallets are discovered from committed activity, not only external leaderboards. |
| HLCOV-WALLET-003 | §15.3 | Tracking tiers are explainable and versioned. |
| HLCOV-WALLET-004 | §15.4, §28.8 | Backfill is resumable, leased, idempotent, and budget-aware. Historical coverage and truncation are visible per wallet/dataset. |
| HLCOV-WALLET-005 | §16.1, §16.2, §16.3 | Position episodes, cashflow-aware PnL decomposition, and performance outputs. Do not treat raw account-value change as investment performance. |
| HLCOV-WALLET-006 | §28.6 | At least 10,000 wallet registry records can be scheduled without exceeding configured source budgets. |
| HLCOV-WALLET-007 | §28.7 | Current wallet positions, orders, and balances reconcile within approved semantics. |
| HLCOV-WALLET-008 | §15.1, §19.4 | Operator watchlists and labels are in scope. Extend the existing watchlist schema. Do not duplicate it. |
| HLCOV-EVM-001 | §17.1 | Local `evm_block_and_receipts` is primary. S3 is fallback/independent. Official JSON-RPC is supplemental. |
| HLCOV-EVM-002 | §17.2 | HyperEVM protocol modules start in `hl-protocol`. A separate Ethereum crate is justified only after dependency/architecture evidence. |
| HLCOV-EVM-003 | §17.3 | Indexed facts include blocks, txs, receipts, logs, system transactions, token events, CoreWriter actions, and Core/EVM transfers. |
| HLCOV-EVM-004 | §17.4 | PostgreSQL ABI/contract registry. Unknown logs remain queryable by topic, address, and raw bytes. |
| HLCOV-EVM-005 | §17.5, §28.12 | Cross-layer links for Core/EVM transfers and known CoreWriter actions. Read precompiles reconcile. They do not replace L1 replay. |
| HLCOV-EVM-006 | §30.12, §28.11 | HyperEVM raw blocks/receipts are first-class canonical chain facts, archived and queryable. |
| HLCOV-ANALYTICS-001 | §19.5, §28.13 | ClickHouse fact projections are rebuildable from immutable archives and are never the only copy. |
| HLCOV-ANALYTICS-002 | §7, §28.13 | Deterministic archive-to-ClickHouse projectors. |
| HLCOV-ANALYTICS-003 | §7, §28.14 | Production `hl-analytics` drives existing wallet/entity/market intelligence from production facts. |
| HLCOV-ANALYTICS-004 | §20.1 | Wallet-intelligence projections listed in §20.1. |
| HLCOV-ANALYTICS-005 | §20.2, §28.15 | Rankings are cashflow-adjusted, point-in-time safe, entity-aware, and coverage-labelled. Never rank purely by raw PnL. |
| HLCOV-ANALYTICS-006 | §20.3 | Market aggregates carry explicit coverage fields. Tracked-wallet statistics are not called global unless committed venue-wide reconstruction supports that. |
| HLCOV-ANALYTICS-007 | §20.4 | Provider labels and leaderboards are temporal observations, separate from reconstructed metrics. |
| HLCOV-ANALYTICS-008 | §21, §28.16 | Alerts are typed, versioned, deterministic, and point-in-time safe, with the §21.2 lifecycle and evidence bundles. Language models do not decide confirmation. |
| HLCOV-API-001 | §22.1 | Analytical responses include schema version, as-of, knowledge time, watermark, confirmation, source coverage, data health, and build. |
| HLCOV-API-002 | §22.2, §22.3, §22.4, §22.5 | Market, wallet, ecosystem, and HyperEVM GET surfaces in §22. |
| HLCOV-API-003 | §22.6 | Operations/evidence endpoints. Control writes are local Alpha Desk metadata only. |
| HLCOV-API-004 | §22.7 | Resumable streams carry sequence, watermark, snapshot/resume token, and stale-state markers. |
| HLCOV-API-005 | §23 | Dashboard workspaces: market, wallet, ecosystem, operations. |
| HLCOV-API-006 | §28.20 | Dashboard exposes data health, source coverage, and limitations alongside intelligence. |
| HLCOV-API-007 | parent adjudication L | Do not create a second web app. Extend existing `apps/AlphaDesk` / operator dashboard. |
| HLCOV-API-008 | §3.2 | No GraphQL-first rewrite of the existing OpenAPI service. |
| HLCOV-OPS-001 | §3.2, §30.14 | Read-only: no `/exchange`, signing, private keys, order placement, copy-trading execution, or vault actions. |
| HLCOV-OPS-002 | §3.2 | No claim of complete historical coverage when the source is truncated or gapped. |
| HLCOV-OPS-003 | §3.2, §30.7 | Rust is the production runtime. Python is not the canonical production path. |
| HLCOV-OPS-004 | §24 | Security, privacy, and legal controls in §24, including egress allowlist, TLS, license field suppression, and label provenance. |
| HLCOV-OPS-005 | §24, §28.18 | Release artifacts contain no execution/signing code or secrets. |
| HLCOV-OPS-006 | §25 | Latency, correctness, and capacity SLOs plus the core metrics in §25.4. Initial numbers are measured, not frozen. |
| HLCOV-OPS-007 | §26 | Protocol, differential, replay, chaos, and research tests, plus the verification commands in §26.6. |
| HLCOV-OPS-008 | §27.1 | Initial production topology in §27.1. |
| HLCOV-OPS-009 | §28.19 | Load, soak, chaos, restore, and replay gates pass before calling the expansion complete. |
| HLCOV-OPS-010 | T01 brief | Design addendum, renamed spec/plan, traceability, plans index, ROADMAP note, and STATUS snapshot. Docs check is `just hyperliquid-full-coverage-docs`. |
| HLCOV-OPS-011 | parent adjudication M | First release profile enables no third-party providers. |
| HLCOV-OPS-012 | §8, T36 | `hlscreen` Alpha Desk API source mode stays in `rsitech-ai/hlscreen`. Observability may ship without that work. |
