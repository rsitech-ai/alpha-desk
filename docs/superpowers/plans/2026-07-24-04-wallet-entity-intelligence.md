# Stage 3 Wallet and Entity Intelligence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce point-in-time, uncertainty-aware account and entity intelligence: performance, whale components, skill, style, intent, copyability, capacity, temporal clusters, independence, leader/follower relationships, counterparty edges, and behavioral change regimes.

**Architecture:** `feature-core` defines immutable bitemporal feature snapshots and online window primitives. `wallet-intelligence` computes account-level metrics from exact state deltas and analytical episodes. `entity-graph` stores hard and probabilistic links as temporal evidence, forms versioned clusters under conservative policies, and emits independence weights. `hl-analytics` materializes historical snapshots in ClickHouse and Parquet while `hl-core` maintains only the bounded online state needed for live calculations.

**Tech Stack:** Rust 1.97.1, Arrow/Parquet, ClickHouse 26.3 LTS, RocksDB rolling-window state, Polars lazy API for reviewed batch jobs, native Rust Bayesian/robust statistics, Petgraph or a narrowly wrapped graph implementation, Proptest, deterministic seeded simulations, no Python production service.

## Global Constraints

- Stage 2 tag `stage-2-state` and its gate record must verify before this plan begins.
- Address, trading account, master/subaccount relation, vault, entity, and cohort are distinct types.
- Deposits and withdrawals are cash flows, never trading profit.
- Every intelligence value is point-in-time and bitemporal; historical output uses the feature, cluster, and behavior version known then.
- Scores expose components, intervals, effective sample size, freshness, applicable markets/horizons/regimes, and data health.
- Small samples are shrunk toward reviewed priors; no opaque letter grades are canonical.
- Soft entity links are probabilistic evidence, not identity claims.
- False merges are treated as more damaging than false splits for consensus calculations.
- Off-platform hedges are unobservable; intent and hedge status are probabilities with uncertainty.
- Copyability is parameterized by latency, bankroll, fees, funding, current book, and portfolio context.
- Learned or statistical outputs that lack sufficient evidence are withheld or marked uncalibrated.
- V1 remains read-only and local-only.
- Every task follows TDD and ends in a focused commit.

---

### Task 1: Define bitemporal feature snapshots and online windows

**Files:**
- Modify: `crates/feature-core/src/lib.rs`
- Create: `crates/feature-core/src/feature.rs`
- Create: `crates/feature-core/src/snapshot.rs`
- Create: `crates/feature-core/src/window.rs`
- Create: `crates/feature-core/src/asof.rs`
- Create: `crates/feature-core/src/errors.rs`
- Create: `crates/feature-core/tests/asof.rs`
- Create: `crates/feature-core/tests/windows.rs`
- Create: `schemas/proto/feature/v1/feature.proto`
- Create: `schemas/clickhouse/0002_feature_snapshots.sql`

**Interfaces:**
- Consumes: `StateDelta`, protocol/known time, feature-set versions, entity/account/market IDs.
- Produces: immutable `FeatureSnapshot`, typed feature keys/values, content-addressed `EvidenceRef`, exact as-of joins, deterministic rolling windows, and ClickHouse feature tables.

- [ ] **Step 1: Verify Stage 2 and write leakage tests**

```bash
git verify-tag stage-2-state
just stage-2-gate
```

Create a test with facts effective at `t1` but learned at `t3`; an as-of query at `t2` must not return them even though `effective_at <= t2`.

```rust
#[test]
fn asof_join_respects_effective_and_known_time() {
    let rows = fixture_rows();
    assert!(asof(&rows, time("t2"), time("t2")).is_none());
    assert_eq!(asof(&rows, time("t2"), time("t3")).unwrap().revision, 1);
}
```

Run `cargo test -p feature-core --test asof`; expect FAIL.

- [ ] **Step 2: Define stable feature identity and values**

```rust
pub struct FeatureKey {
    pub namespace: String,
    pub name: String,
    pub version: u32,
}

pub enum FeatureSubject {
    Account(AccountId),
    Entity(EntityId),
    Market(MarketId),
    AccountMarket { account_id: AccountId, market_id: MarketId },
    EntityMarket { entity_id: EntityId, market_id: MarketId },
}

pub enum MissingReason {
    NotObserved,
    InsufficientHistory,
    Unsupported,
    NotApplicable,
    RedDataHealth,
}

pub enum FeatureValue {
    Decimal { raw: i128, scale: u32 },
    SignedInteger(i64),
    UnsignedInteger(u64),
    ProbabilityPpm(ProbabilityPpm),
    Category(String),
    Boolean(bool),
    Missing(MissingReason),
}

pub enum EvidenceKind {
    CanonicalEvent,
    StateSnapshot,
    BookSnapshot,
    FeatureSnapshot,
    OperatorAnnotation,
    ModelArtifact,
}

pub struct EvidenceRef {
    pub kind: EvidenceKind,
    pub evidence_id: EvidenceId,
    pub content_hash: [u8; 32],
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
}

pub struct FeatureSnapshot {
    pub subject: FeatureSubject,
    pub feature_set_version: FeatureSetVersion,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub superseded_at: Option<KnownTime>,
    pub revision: u32,
    pub values: BTreeMap<FeatureKey, FeatureValue>,
    pub input_watermark: BlockHeight,
    pub data_health: HealthState,
    pub provenance_hash: [u8; 32],
}
```

`EvidenceRef::try_new` rejects an empty `evidence_id` and an all-zero content hash. Evidence references identify heterogeneous immutable inputs without creating dependencies from `feature-core` back into state, archive, graph, or model crates. Feature keys are registered in versioned manifests; renaming or changing meaning creates a new version.

- [ ] **Step 3: Implement deterministic windows**

Provide event-count, protocol-time, exponentially weighted, quantile-sketch, covariance, and robust z-score windows. Updates accept explicit event time and sequence; duplicate event IDs are ignored. Serialization includes algorithm and parameter version.

- [ ] **Step 4: Implement bitemporal ClickHouse tables**

Tables order by `(feature_set_version, subject_type, subject_id, effective_at, known_at, revision)` and append corrections. Views must require an explicit `as_of_known_at` parameter rather than silently using latest knowledge for historical research.

- [ ] **Step 5: Verify windows, serialization, and as-of queries**

```bash
cargo test -p feature-core
clickhouse-client --multiquery < schemas/clickhouse/0002_feature_snapshots.sql
```

Expected: property tests show chunked versus one-pass updates produce identical snapshots.

- [ ] **Step 6: Commit**

```bash
git add crates/feature-core schemas/proto/feature schemas/clickhouse/0002_feature_snapshots.sql Cargo.toml Cargo.lock
git commit -m "feat(features): add bitemporal snapshots and deterministic windows"
```

---

### Task 2: Implement the account performance ledger and whale taxonomy components

**Files:**
- Modify: `crates/wallet-intelligence/src/lib.rs`
- Create: `crates/wallet-intelligence/src/performance.rs`
- Create: `crates/wallet-intelligence/src/cashflows.rs`
- Create: `crates/wallet-intelligence/src/markout.rs`
- Create: `crates/wallet-intelligence/src/risk.rs`
- Create: `crates/wallet-intelligence/src/whale.rs`
- Create: `crates/wallet-intelligence/tests/performance.rs`
- Create: `crates/wallet-intelligence/tests/whale.rs`
- Create: `schemas/clickhouse/0003_wallet_features.sql`
- Create: `docs/metrics/wallet-performance.md`

**Interfaces:**
- Consumes: exact account state, ledger events, position episodes, market returns, fills, book estimates, and regimes when available.
- Produces: point-in-time performance snapshots, risk statistics, markouts, beta/concentration metrics, and visible whale components.

- [ ] **Step 1: Write cash-flow-adjusted return tests**

Create an account that starts with $100, gains $10, receives a $100 deposit, and ends at $215. Assert trading gain is $15, not $115, and the time-weighted return links subperiod returns without treating the deposit as profit.

- [ ] **Step 2: Implement performance event sourcing**

`PerformanceLedger` records starting/ending equity, external cash flows, realized/unrealized PnL, fees, funding, gross exposure, capital at risk, and observations per protocol interval. It emits time-weighted return, money-weighted return only when mathematically meaningful, drawdown, recovery duration, expected shortfall, downside deviation, turnover, and utilization.

- [ ] **Step 3: Implement multi-horizon markouts and attribution**

For entries and exits, compute executable or observed markout at configured horizons using only prices known after the label horizon. Store maker/taker role, side, market, regime, fee/funding attribution, and whether the horizon outcome is complete.

- [ ] **Step 4: Implement visible whale components**

```rust
pub struct WhaleComponents {
    pub capital_percentile: ProbabilityPpm,
    pub position_oi_share: MarginRatio,
    pub flow_volume_share: MarginRatio,
    pub impact_depth_ratio_25bps: MarginRatio,
    pub account_commitment: MarginRatio,
    pub forced_flow_potential: MarginRatio,
    pub influence_score: Option<ProbabilityPpm>,
    pub skill_probability: Option<ProbabilityPpm>,
    pub fragility_score: Option<ProbabilityPpm>,
}
```

Normalize by asset and regime using reviewed robust location/scale estimators. Do not combine these into a hidden canonical score.

- [ ] **Step 5: Add concentration and single-winner dependence tests**

Test asset/DEX/collateral/regime concentration, best trade/month share, long/short beta, maker/taker mix, and performance before/after capital changes. Run:

```bash
cargo test -p wallet-intelligence performance whale
```

Expected: PASS and exact decimal preservation.

- [ ] **Step 6: Commit**

```bash
git add crates/wallet-intelligence schemas/clickhouse/0003_wallet_features.sql docs/metrics/wallet-performance.md Cargo.toml Cargo.lock
git commit -m "feat(wallet): add performance ledger and whale components"
```

---

### Task 3: Implement hierarchical skill posteriors and current relevance

**Files:**
- Modify: `crates/wallet-intelligence/src/lib.rs`
- Create: `crates/wallet-intelligence/src/skill/mod.rs`
- Create: `crates/wallet-intelligence/src/skill/posterior.rs`
- Create: `crates/wallet-intelligence/src/skill/effective_sample.rs`
- Create: `crates/wallet-intelligence/src/skill/priors.rs`
- Create: `crates/wallet-intelligence/src/skill/relevance.rs`
- Create: `crates/wallet-intelligence/tests/skill.rs`
- Create: `config/models/wallet-skill-v1.toml`
- Create: `docs/models/wallet-skill-v1.md`

**Interfaces:**
- Consumes: net markout outcomes, binary outcome summaries, market/horizon/regime taxonomy, temporal weights, and data-health state.
- Produces: a versioned statistical `SkillVector` with posterior mean, credible interval, probability of positive net edge, effective sample size, freshness, and applicability. Task 5 composes it with copyability and capacity into the public `WalletIntelligenceVector` required by the approved design.

- [ ] **Step 1: Write shrinkage and autocorrelation tests**

Assert a wallet with 3 positive observations is shrunk more strongly than one with 300. Assert 100 highly autocorrelated observations have lower effective sample size than 100 independent observations. Assert stale evidence reduces current relevance without altering historical posterior snapshots.

- [ ] **Step 2: Define the public skill contract**

```rust
pub enum IntelligenceSubject {
    Account(AccountId),
    Entity(EntityId),
}

pub enum ApplicabilitySupport {
    Supported,
    InsufficientEvidence,
    Unsupported,
}

pub struct Applicability {
    pub markets: Vec<MarketId>,
    pub horizons: Vec<Horizon>,
    pub regimes: Vec<RegimeId>,
    pub support: ApplicabilitySupport,
    pub reason_codes: Vec<String>,
}

pub struct SkillEstimate {
    pub posterior_mean_bps: BasisPoints,
    pub credible_interval_bps: ClosedInterval<BasisPoints>,
    pub probability_positive: ProbabilityPpm,
    pub effective_sample_size_milli: u64,
    pub freshness: ProbabilityPpm,
    pub calibration: CalibrationStatus,
    pub applicability: Applicability,
}

pub struct SkillVector {
    pub directional: SkillEstimate,
    pub entry_timing: SkillEstimate,
    pub exit_timing: SkillEstimate,
    pub execution: SkillEstimate,
    pub market_making: SkillEstimate,
    pub carry: SkillEstimate,
    pub risk_discipline: SkillEstimate,
    pub consistency: SkillEstimate,
    pub regime_fit: SkillEstimate,
    pub current_relevance: SkillEstimate,
}
```

This `SkillVector` is the statistical subvector. Copyability and capacity are execution-dependent components calculated in Task 5 and attached to the public `WalletIntelligenceVector`; they are not falsely represented as Bayesian markout posteriors.

- [ ] **Step 3: Implement reviewed Bayesian estimators in Rust**

Start with a robust normal-inverse-gamma equivalent for net markouts, beta-binomial only for explicitly binary diagnostics, hierarchical prior lookup by market/horizon/regime, and block-bootstrap-adjusted effective sample size. Every parameter is versioned in `wallet-skill-v1.toml`.

- [ ] **Step 4: Implement relevance decay and segment boundaries**

Evidence weights use explicit protocol time. Change-point segments supplied by Task 9 prevent blind pooling. The posterior exposes `insufficient_evidence` when policy minimums are not met.

- [ ] **Step 5: Validate against simulation and closed-form fixtures**

Use seeded simulations with known mean/variance, heavy tails, and regime breaks. Compare simple cases to an independently reviewed calculation. Run:

```bash
cargo test -p wallet-intelligence skill
cargo run -p hl-analytics -- validate-wallet-skill fixtures/models/wallet-skill-v1
```

Expected posterior coverage and calibration fall within bounds documented in the model card.

- [ ] **Step 6: Commit**

```bash
git add crates/wallet-intelligence/src/skill crates/wallet-intelligence/tests/skill.rs config/models/wallet-skill-v1.toml docs/models/wallet-skill-v1.md
git commit -m "feat(wallet): add uncertainty-aware skill posteriors"
```

---

### Task 4: Implement temporal trading-style and intent probabilities

**Files:**
- Create: `crates/wallet-intelligence/src/style.rs`
- Create: `crates/wallet-intelligence/src/intent.rs`
- Create: `crates/wallet-intelligence/src/hedge.rs`
- Create: `crates/wallet-intelligence/tests/style_intent.rs`
- Create: `config/models/style-intent-v1.toml`
- Create: `docs/models/style-intent-v1.md`
- Create: `fixtures/models/style-intent-v1/directional-swing.json`
- Create: `fixtures/models/style-intent-v1/market-maker.json`
- Create: `fixtures/models/style-intent-v1/carry.json`
- Create: `fixtures/models/style-intent-v1/follower.json`
- Create: `fixtures/models/style-intent-v1/liquidation-response.json`
- Create: `fixtures/models/style-intent-v1/ambiguous-hedged.json`

**Interfaces:**
- Consumes: turnover, maker ratio, order resting behavior, inventory dynamics, directional beta, hold periods, funding sensitivity, spot/perp offsets, synchronized activity, response lag, liquidation flags, and transfers.
- Produces: versioned probability vectors for trading style and per-position-change intent, plus hedge likelihood and external-hedge uncertainty.

- [ ] **Step 1: Write probability and temporal-version tests**

Assert probabilities sum to exactly 1,000,000 ppm after deterministic rounding; missing critical inputs increase `unclassified_mixed` or `unknown` rather than being imputed silently. A style change at block 500 must not alter snapshots at block 499.

- [ ] **Step 2: Implement interpretable V1 classifiers**

Use deterministic rules plus regularized multinomial/logistic coefficients stored in reviewed configuration. Define the categories exactly:

```rust
pub enum StyleClass {
    DirectionalDiscretionary,
    MomentumTrend,
    MeanReversion,
    Scalping,
    SwingTrading,
    MarketMaking,
    BasisSpotPerpArbitrage,
    FundingCarryCapture,
    LiquidationTrading,
    PortfolioHedge,
    VaultStrategy,
    AutomatedFollower,
    UnclassifiedMixed,
}

pub enum IntentClass {
    OpenDirectional,
    AddDirectional,
    ReduceRisk,
    CloseDirectional,
    HedgeExistingExposure,
    CarryOrBasis,
    MarketMakerInventory,
    LiquidationOrForced,
    TransferOrAccountRebalance,
    Unknown,
}
```

Each output includes feature contributions, support status, and calibration status.

- [ ] **Step 3: Implement hedge-likelihood evidence**

Features include opposing spot/perp exposure, correlated opposite positions, synchronized changes, funding sensitivity, low net beta despite gross turnover, market-making inventory reversion, and linked-account activity. Output:

```rust
pub struct HedgeAssessment {
    pub on_platform_hedge_probability: ProbabilityPpm,
    pub external_hedge_uncertainty: ProbabilityPpm,
    pub evidence: Vec<EvidenceRef>,
    pub limitations: Vec<String>,
}
```

- [ ] **Step 4: Build and review a labeled audit set**

The fixture set includes clear market makers, trend followers, scalpers, basis/carry behavior, liquidations, risk reductions, followers, and mixed/ambiguous accounts. Labels include reviewer disagreement rather than forcing certainty.

- [ ] **Step 5: Run calibration and regression tests**

```bash
cargo test -p wallet-intelligence style_intent
cargo run -p hl-analytics -- evaluate-style-intent fixtures/models/style-intent-v1
```

Expected: probability normalization, stable feature contributions, and minimum calibration metrics documented in the model card.

- [ ] **Step 6: Commit**

```bash
git add crates/wallet-intelligence/src/style.rs crates/wallet-intelligence/src/intent.rs crates/wallet-intelligence/src/hedge.rs crates/wallet-intelligence/tests/style_intent.rs config/models/style-intent-v1.toml docs/models/style-intent-v1.md fixtures/models/style-intent-v1
git commit -m "feat(wallet): classify temporal style intent and hedge likelihood"
```

---

### Task 5: Implement personalized copyability and capacity curves

**Files:**
- Create: `crates/wallet-intelligence/src/copyability.rs`
- Create: `crates/wallet-intelligence/src/capacity.rs`
- Create: `crates/wallet-intelligence/tests/copyability.rs`
- Create: `schemas/proto/intelligence/v1/wallet.proto`
- Create: `docs/metrics/copyability.md`

**Interfaces:**
- Consumes: wallet action timing/ladder/order type, skill half-life, current/historical books, execution estimates, hold/exit behavior, follower latency, bankroll, fees, funding, crowding, and portfolio overlap.
- Produces: P10/P50/P90 follower net return, fill probability, alpha remaining, maximum notional by cost threshold, and copyability class.

- [ ] **Step 1: Write latency and bankroll sensitivity tests**

Use a fixture in which the originator has positive markout at 250 ms but negative net return at four seconds. Assert the class changes from `LatencySensitive` to `NotCopyable`. Increase bankroll until market impact exceeds policy; assert `CapacityLimited` and lower maximum notional.

- [ ] **Step 2: Define the request and result**

```rust
pub struct PortfolioContextSummary {
    pub gross_exposure: UsdAmount,
    pub net_exposure: UsdAmount,
    pub same_market_exposure: UsdAmount,
    pub same_entity_exposure: UsdAmount,
    pub correlated_exposure: UsdAmount,
    pub snapshot_hash: [u8; 32],
}

pub struct CopyabilityRequest {
    pub subject: IntelligenceSubject,
    pub action_id: EventId,
    pub detection_latency: LatencyDistribution,
    pub bankroll: UsdAmount,
    pub max_participation: ProbabilityPpm,
    pub fee_schedule_id: FeeScheduleId,
    pub portfolio_context: PortfolioContextSummary,
}

pub enum CopyabilityClass {
    NotCopyable,
    LatencySensitive,
    CapacityLimited,
    ResearchOnly,
    Actionable,
}
```

The result includes all requested outputs plus data/book health and assumptions hash. Define the public aggregate contract in `schemas/proto/intelligence/v1/wallet.proto` and Rust:

```rust
pub struct CopyabilitySummary {
    pub class: CopyabilityClass,
    pub p10_net_return_bps: BasisPoints,
    pub p50_net_return_bps: BasisPoints,
    pub p90_net_return_bps: BasisPoints,
    pub fill_probability: ProbabilityPpm,
    pub alpha_remaining: ProbabilityPpm,
    pub assumptions_hash: [u8; 32],
}

pub struct CapacitySummary {
    pub maximum_notional: UsdAmount,
    pub cost_threshold_bps: BasisPoints,
    pub stressed_maximum_notional: UsdAmount,
    pub book_as_of_block: BlockHeight,
    pub health: HealthAssessment,
}

pub struct WalletIntelligenceVector {
    pub statistical_skill: SkillVector,
    pub copyability: CopyabilitySummary,
    pub capacity: CapacitySummary,
    pub feature_set_version: FeatureSetVersion,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub input_watermark: BlockHeight,
}
```

This public aggregate satisfies the approved vector dimensions `copyability` and `capacity` while retaining their distinct assumptions and provenance.

- [ ] **Step 3: Implement alpha-decay and capacity curves**

Fit reviewed monotone or survival-based half-life estimators by wallet/market/action class; fall back to conservative cohort estimates for sparse samples. Capacity walks current or historical books through the `orderbook` execution interface and includes exit stress.

- [ ] **Step 4: Add conservative missing-data behavior**

Red book health returns no actionable estimate. Missing fee/funding or sparse half-life evidence returns `ResearchOnly` with explicit reasons. Portfolio correlation may reduce utility but does not rewrite canonical wallet skill.

- [ ] **Step 5: Verify and document**

```bash
cargo test -p wallet-intelligence copyability
```

Expected: monotonic capacity curves, worse or equal net return with greater latency/cost/size under controlled fixtures, and exact assumption hashes.

- [ ] **Step 6: Commit**

```bash
git add crates/wallet-intelligence/src/copyability.rs crates/wallet-intelligence/src/capacity.rs crates/wallet-intelligence/tests/copyability.rs schemas/proto/intelligence/v1/wallet.proto docs/metrics/copyability.md
git commit -m "feat(wallet): estimate executable copyability and capacity"
```

---

### Task 6: Implement hard-link graph and temporal cluster versions

**Files:**
- Modify: `crates/entity-graph/src/lib.rs`
- Create: `crates/entity-graph/src/node.rs`
- Create: `crates/entity-graph/src/evidence.rs`
- Create: `crates/entity-graph/src/hard_link.rs`
- Create: `crates/entity-graph/src/cluster.rs`
- Create: `crates/entity-graph/src/version.rs`
- Create: `crates/entity-graph/tests/hard_links.rs`
- Create: `schemas/clickhouse/0004_entity_graph.sql`
- Create: `schemas/postgres/0002_entity_annotations.sql`
- Create: `docs/models/entity-link-policy-v1.md`

**Interfaces:**
- Consumes: protocol master/subaccount, vault-management, explicit internal-transfer semantics, and reviewed operator annotations.
- Produces: immutable hard-link evidence, known administrative groups, temporal cluster versions, and as-of membership queries.

- [ ] **Step 1: Write temporal cluster tests**

A subaccount relation learned at block 1,000/known at 1,002 must not appear in an as-of query known at 1,001. A verified annotation may create a new version but must not rewrite prior versions.

- [ ] **Step 2: Define evidence and cluster contracts**

```rust
pub enum GraphNodeId {
    Account(AccountId),
    MasterAccount(MasterAccountId),
    Vault(VaultId),
    Entity(EntityId),
}

pub enum LinkKind {
    ProtocolSubaccount,
    ProtocolVaultMembership,
    ProtocolVaultManager,
    ApprovedOperatorAnnotation,
    FundingPath,
    CoordinatedExecution,
    SizePriceFingerprint,
    LeaderFollower,
    CounterpartyInventoryHandoff,
    StrategyMigration,
}

pub struct LinkEvidence {
    pub evidence_id: EvidenceId,
    pub left: GraphNodeId,
    pub right: GraphNodeId,
    pub kind: LinkKind,
    pub probability: ProbabilityPpm,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub source_refs: Vec<EvidenceRef>,
    pub reviewer: Option<String>,
}

pub struct ClusterMembershipVersion {
    pub cluster_version_id: ClusterVersionId,
    pub entity_id: EntityId,
    pub member: AccountId,
    pub weight: ProbabilityPpm,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub superseded_at: Option<KnownTime>,
}
```

- [ ] **Step 3: Implement conservative hard-link aggregation**

Only protocol hard links or approved annotations can create a `KnownAdministrativeGroup`. PnL remains available at both account and group levels. The graph stores direction and semantics; repeated transfers without protocol semantics remain soft evidence.

- [ ] **Step 4: Implement versioning and audit history**

Every cluster change appends a version with evidence set hash, policy version, build ID, and reviewer/automation source. Deletion is a superseding record, not destructive mutation.

- [ ] **Step 5: Verify ClickHouse/PostgreSQL migrations and as-of queries**

```bash
cargo test -p entity-graph hard_links
clickhouse-client --multiquery < schemas/clickhouse/0004_entity_graph.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schemas/postgres/0002_entity_annotations.sql
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/entity-graph schemas/clickhouse/0004_entity_graph.sql schemas/postgres/0002_entity_annotations.sql docs/models/entity-link-policy-v1.md Cargo.toml Cargo.lock
git commit -m "feat(entity): add hard links and temporal cluster versions"
```

---

### Task 7: Implement soft-link evidence, conservative clustering, and independence weights

**Files:**
- Create: `crates/entity-graph/src/soft_link.rs`
- Create: `crates/entity-graph/src/features.rs`
- Create: `crates/entity-graph/src/policy.rs`
- Create: `crates/entity-graph/src/independence.rs`
- Create: `crates/entity-graph/tests/soft_links.rs`
- Create: `crates/entity-graph/tests/false_merge.rs`
- Create: `config/models/entity-link-policy-v1.toml`
- Create: `fixtures/models/entity-links-v1/hard-subaccount.json`
- Create: `fixtures/models/entity-links-v1/hard-vault.json`
- Create: `fixtures/models/entity-links-v1/transfer-soft-link.json`
- Create: `fixtures/models/entity-links-v1/coordinated-execution.json`
- Create: `fixtures/models/entity-links-v1/independent-control.json`
- Create: `fixtures/models/entity-links-v1/false-merge-adversarial.json`

**Interfaces:**
- Consumes: capital paths, synchronized actions, size/price fingerprints, market selection, timing, shared counterparties, inventory handoffs, and strategy migration evidence.
- Produces: probabilistic soft edges, policy-approved temporal clusters, alternatives, and normalized effective independence weights.

- [ ] **Step 1: Write false-merge and address-fanout tests**

Construct 20 follower addresses controlled by one synthetic entity and 5 independent traders. Assert effective consensus is approximately 6, not 25. Construct two independent high-frequency traders with coincidental timing; conservative policy must keep them split unless multiple independent evidence families cross threshold.

- [ ] **Step 2: Implement evidence-family features**

Each soft edge stores separate probabilities for funding path, synchronization beyond market coincidence, size/price fingerprint, follower lag, unusual market overlap, counterparty/inventory handoff, and migration. The aggregate model cannot hide individual evidence families.

- [ ] **Step 3: Implement conservative clustering policy**

Policy requires configurable minimum distinct evidence families, posterior threshold, stability duration, and false-merge review sample. Accounts below aggregation threshold remain linked but separate. Cluster output includes alternative partition likelihood where material.

- [ ] **Step 4: Implement independence weights**

```rust
pub fn independence_weight(input: &IndependenceInput) -> ProbabilityPpm {
    product_ppm([
        input.hard_cluster_share,
        complement(input.follower_probability),
        complement(input.coordinated_action_probability),
        input.evidence_quality,
    ])
}
```

Normalize weights within the cohort so a likely entity contributes approximately one vote while retaining uncertainty. Use exact integer probability arithmetic.

- [ ] **Step 5: Evaluate the reviewed link set**

Report precision/recall where labels exist, false-merge rate, false-split diagnostics, cluster stability, and consensus sensitivity at multiple thresholds. The approved policy prioritizes false-merge control.

Run:

```bash
cargo test -p entity-graph soft_links false_merge
cargo run -p hl-analytics -- evaluate-entity-links fixtures/models/entity-links-v1
```

Expected: metrics meet the bounds documented in policy.

- [ ] **Step 6: Commit**

```bash
git add crates/entity-graph/src/soft_link.rs crates/entity-graph/src/features.rs crates/entity-graph/src/policy.rs crates/entity-graph/src/independence.rs crates/entity-graph/tests config/models/entity-link-policy-v1.toml fixtures/models/entity-links-v1
git commit -m "feat(entity): infer conservative links and independence weights"
```

---

### Task 8: Implement leader/follower and counterparty intelligence

**Files:**
- Create: `crates/entity-graph/src/leader_follower.rs`
- Create: `crates/entity-graph/src/counterparty.rs`
- Create: `crates/entity-graph/src/markout_control.rs`
- Create: `crates/entity-graph/tests/leader_follower.rs`
- Create: `crates/entity-graph/tests/counterparty.rs`
- Create: `schemas/clickhouse/0005_relationship_features.sql`
- Create: `docs/metrics/leader-follower.md`
- Create: `docs/metrics/counterparty-intelligence.md`

**Interfaces:**
- Consumes: material action events, market movement controls, maker/taker role, sizes, entry prices, entity versions, and forward markouts.
- Produces: temporal relationship edges, lag distributions, follower probability, originator/independent/contrarian classifications, adverse-selection and inventory-transfer metrics.

- [ ] **Step 1: Write market-movement-control tests**

Simulate many accounts reacting independently to one market jump; simple timestamp correlation is high, but conditional follower probability after controlling for market movement must remain low. Simulate one account consistently following another at a stable lag; classification must identify the leader/follower relation.

- [ ] **Step 2: Implement event-history pair features**

For candidate pairs, compute directional action lag distribution, conditional similar-action probability, size relationship, market overlap, entry degradation, independent predictive value, and edge decay. Use bounded candidate generation to avoid all-pairs explosion.

- [ ] **Step 3: Implement reviewed relationship classification**

Categories exactly match: `Originator`, `IndependentConfirmer`, `FastFollower`, `SlowFollower`, `CopyBot`, `ContrarianResponder`, and `NoStableRelation`. Each edge carries posterior probability, sample size, validity interval, evidence refs, and model version.

- [ ] **Step 4: Implement counterparty controls**

Compute A-versus-B markout, passive adverse selection, repeated inventory transfer, price-discovery initiation, profitable exits into follower cohorts, and maker toxicity by market/regime. Control for market direction and maker/taker role through matched or regression-adjusted baselines.

- [ ] **Step 5: Verify temporal and statistical behavior**

```bash
cargo test -p entity-graph leader_follower counterparty
```

Expected: no future events enter the feature window, relationship versions do not rewrite history, and independent market reactions are not labeled copying.

- [ ] **Step 6: Commit**

```bash
git add crates/entity-graph/src/leader_follower.rs crates/entity-graph/src/counterparty.rs crates/entity-graph/src/markout_control.rs crates/entity-graph/tests schemas/clickhouse/0005_relationship_features.sql docs/metrics/leader-follower.md docs/metrics/counterparty-intelligence.md
git commit -m "feat(entity): add leader follower and counterparty intelligence"
```

---

### Task 9: Implement behavioral change regimes and intelligence materialization

**Files:**
- Create: `crates/wallet-intelligence/src/change_point.rs`
- Create: `crates/wallet-intelligence/src/behavior_regime.rs`
- Create: `crates/wallet-intelligence/tests/change_point.rs`
- Create: `services/hl-analytics/src/wallet/mod.rs`
- Create: `services/hl-analytics/src/wallet/online.rs`
- Create: `services/hl-analytics/src/wallet/batch.rs`
- Create: `services/hl-analytics/src/entity/mod.rs`
- Create: `services/hl-analytics/tests/intelligence_pipeline.rs`
- Create: `config/models/change-point-v1.toml`
- Create: `infra/monitoring/dashboards/wallet-entity.json`

**Interfaces:**
- Consumes: state deltas, episodes, performance, style/intent, capital changes, graph evidence, and historical archives.
- Produces: new behavior regimes, dormant/capital/risk/strategy alerts, online feature updates, batch corrections, ClickHouse/Parquet snapshots, and feature health.

- [ ] **Step 1: Write seeded change-point tests**

Generate stable maker behavior followed by a directional taker regime, dormant reactivation, leverage escalation, and skill decay. Assert change points fall within reviewed detection tolerance and prior snapshots remain unchanged.

- [ ] **Step 2: Implement bounded online change detection**

Use a reviewed Bayesian online change-point or CUSUM/robust alternative with explicit hazard and minimum evidence. Emit reason components for capital, turnover, maker ratio, market specialization, leverage, risk escalation, and linked-account migration.

- [ ] **Step 3: Integrate online and batch paths through the same calculators**

Online workers update bounded windows on each `StateDelta`; batch workers replay exact deltas through the same `FeatureCalculator` implementations. Batch corrections append new bitemporal revisions rather than mutating history.

- [ ] **Step 4: Materialize intelligence outputs**

Write `wallet_feature_snapshots`, `entity_feature_snapshots`, cluster versions, relationship edges, and behavior regimes to ClickHouse and Parquet. Publish compact online updates to `hl.v1.feature.wallet` and `hl.v1.feature.entity` only after persistence.

- [ ] **Step 5: Run live-vs-replay equivalence tests**

```bash
just dev-up
cargo test -p hl-analytics --test intelligence_pipeline
just dev-down
```

Expected: online and batch snapshots have identical content/provenance hashes for the same event range.

- [ ] **Step 6: Commit**

```bash
git add crates/wallet-intelligence/src/change_point.rs crates/wallet-intelligence/src/behavior_regime.rs crates/wallet-intelligence/tests/change_point.rs services/hl-analytics/src/wallet services/hl-analytics/src/entity services/hl-analytics/tests/intelligence_pipeline.rs config/models/change-point-v1.toml infra/monitoring/dashboards/wallet-entity.json
git commit -m "feat(intelligence): materialize temporal wallet and entity features"
```

---

### Task 10: Execute the Stage 3 wallet/entity gate and manual audit

**Files:**
- Create or modify before verification: `config/stage-gates/stage-3.toml`
- Create before verification: `tests/regression/intelligence/manifest.toml`
- Create or modify before verification: `docs/reviews/wallet-entity-audit-v1.md`
- Create or modify before verification: `justfile`
- Generate after verification: `docs/stage-gates/stage-3-wallet-entity.evidence.json`
- Generate after verification: `docs/stage-gates/stage-3-wallet-entity.md`

**Interfaces:**
- Consumes: the complete stage implementation, approved point-in-time regression material, and prior signed gate evidence.
- Produces: a clean-commit canonical gate report, signed approval record, and signed `stage-3-intelligence` tag.

- [ ] **Step 1: Freeze the regression and review inputs**

Freeze profitable and unprofitable, sparse and dense, maker, directional, carry, follower, liquidation-driven, dormant/reactivated, hard-linked, likely-linked, and deliberately independent subjects. Record labels, uncertainty, reviewer agreement, temporal cutoffs, and source-evidence hashes.

- [ ] **Step 2: Implement the exact gate configuration and tests**

`just stage-3-gate` writes only to ignored `target/stage-gates/stage-3.json` and runs performance/cash-flow/markout properties; posterior coverage and calibration simulations; style/intent normalization and audit metrics; copyability latency/capacity monotonicity; as-of leakage tests; cluster version and false-merge tests; leader/follower market-control tests; counterparty controls; change-point tolerance; online/batch equality; and manual-audit completeness.

The gate runner must reject a dirty worktree before any check, record the clean implementation SHA, and fail closed on missing evidence or approvals. Add a configuration test proving every required command and artifact is present.

- [ ] **Step 3: Commit every gate input before verification**

```bash
git add config/stage-gates/stage-3.toml tests/regression/intelligence docs/reviews/wallet-entity-audit-v1.md justfile
git commit -m "chore(gate): add Stage 3 wallet and entity intelligence verification inputs"
test -z "$(git status --porcelain)"
git rev-parse HEAD
```

The printed SHA is the immutable implementation commit evaluated by this gate.

- [ ] **Step 4: Run the gate from fresh clean clones on two supported hosts**

```bash
just stage-3-gate
cargo run -p intelligence-audit -- tests/regression/intelligence/manifest.toml --output target/stage-gates/stage-3-audit.json
sha256sum target/stage-gates/stage-3.json
```

Expected: PASS; canonical report, state/output hashes, and configured reproducibility views agree across hosts. Host-specific provenance remains recorded but is excluded only from the explicitly defined cross-host comparison projection.

- [ ] **Step 5: Commit evidence, collect approvals, and sign the stage tag**

```bash
cp target/stage-gates/stage-3.json docs/stage-gates/stage-3-wallet-entity.evidence.json
cargo run -p stage-gate -- render-record --evidence docs/stage-gates/stage-3-wallet-entity.evidence.json --output docs/stage-gates/stage-3-wallet-entity.md
git add docs/stage-gates/stage-3-wallet-entity.evidence.json docs/stage-gates/stage-3-wallet-entity.md
git commit -m "docs(gate): record Stage 3 wallet and entity intelligence evidence"
git tag -s stage-3-intelligence -m "Stage 3 wallet and entity intelligence gate passed"
git verify-tag stage-3-intelligence
```

Platform/data, research, and independent reviewers must provide the detached approval artifacts referenced by the record. Do not create the tag when a required check, comparison, review, or bounded-limitation statement is missing.

## Stage 3 Exit Criteria

- Historical performance, skill, style, intent, clusters, relationships, and regimes use only information known at the evaluated time.
- Deposits/withdrawals are excluded from trading PnL and return tests pass.
- Skill outputs expose uncertainty, effective sample size, freshness, and applicability.
- Copyability and capacity respond conservatively to latency, size, cost, book health, and missing evidence.
- Cluster false-merge policy is within approved bounds and consensus uses independence weights.
- Online and replay materializations match exactly.
- Manual audit set is reviewed and `stage-3-intelligence` is approved/tagged.
