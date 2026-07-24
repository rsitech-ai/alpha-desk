# Stage 4 Market Intelligence and Signals Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert exact state plus wallet/entity intelligence into an explainable market-sentiment vector, liquidation-fragility scenarios, historical analogues, and three evidence-complete V1 signal families.

**Architecture:** `market-intelligence` computes scoped cohort metrics and deterministic/transparent statistical features from bitemporal account, entity, book, and market inputs. `signal-core` owns immutable signal objects, append-only lifecycle transitions, evidence completeness, invalidation, utility, deduplication, and outcome hooks. Live and replay evaluations invoke identical feature and signal code.

**Tech Stack:** Rust 1.97.1, exact fixed-point/probability types, Arrow/Parquet, ClickHouse 26.3 LTS, RocksDB bounded online windows, native Rust robust statistics and interpretable hidden-state/change-point regime model, deterministic scenario simulation, HNSW or exact-neighbor adapter behind a stable port for market memory, Proptest, Criterion.

## Global Constraints

- Stage 3 tag `stage-3-intelligence` and its gate record must verify before this plan begins.
- Market sentiment is a vector; no canonical single “bullish/bearish” number may hide its dimensions.
- A complete-market gross long/short notional ratio is not presented as directional truth.
- Every ratio identifies cohort, horizon, unit, denominator, exclusions, effective independent sample, confidence, and watermark.
- Existing static positions are not fresh flow.
- Market-maker, carry, hedge, follower, and forced-liquidation intent are interpreted separately from directional opening risk.
- Entity independence weights are mandatory in consensus and flow aggregation.
- Fragility uses the correct account/margin adapter and refuses exact claims where inputs are estimated.
- A red book, margin model, source, state, feature, or model dependency suppresses the affected signal.
- Every live signal requires a complete evidence bundle and explicit invalidation rules.
- Only three V1 signal families may enter production: independent smart-flow acceleration, smart-versus-crowd divergence, and liquidation-fragility asymmetry.
- Historical analogue retrieval is descriptive evidence, not a prediction by itself.
- V1 remains read-only.
- Every task follows TDD and ends in a focused commit.

---

### Task 1: Define market feature, cohort, and scoped-ratio contracts

**Files:**
- Modify: `crates/market-intelligence/src/lib.rs`
- Create: `crates/market-intelligence/src/sentiment.rs`
- Create: `crates/market-intelligence/src/cohort.rs`
- Create: `crates/market-intelligence/src/ratio.rs`
- Create: `crates/market-intelligence/src/errors.rs`
- Create: `crates/market-intelligence/tests/ratios.rs`
- Create: `schemas/proto/intelligence/v1/market.proto`
- Create: `schemas/clickhouse/0006_market_features.sql`
- Create: `schemas/postgres/0003_cohort_definitions.sql`
- Create: `docs/metrics/market-sentiment.md`

**Interfaces:**
- Consumes: market/account/entity feature snapshots, positions, actions, independence, and data health.
- Produces: `MarketSentimentVector`, immutable cohort definitions, scoped ratio results, effective sample size, and versioned market feature snapshots.

- [ ] **Step 1: Verify Stage 3 and write ratio semantic tests**

```bash
git verify-tag stage-3-intelligence
just stage-3-gate
```

Create a market with one $100 million short entity and 5,000 small long entities. Assert entity-count ratio and cohort exposure ratio differ and both expose their denominator. Assert the API/domain has no function named `venue_gross_long_short_ratio`.

- [ ] **Step 2: Define the sentiment vector**

```rust
pub enum DimensionUnit {
    Usd,
    BasisPoints,
    ProbabilityPpm,
    Count,
    Ratio,
    StandardizedScore,
}

pub struct ScoredDimension {
    pub raw_value: Decimal,
    pub raw_unit: DimensionUnit,
    pub normalized_value: Decimal,
    pub interval: ClosedInterval<Decimal>,
    pub effective_sample_size_milli: u64,
    pub health: HealthAssessment,
    pub feature_refs: Vec<EvidenceRef>,
}

pub struct MarketFeatureSnapshot {
    pub market_id: MarketId,
    pub horizon: Horizon,
    pub feature_set_version: FeatureSetVersion,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub input_watermark: BlockHeight,
    pub values: BTreeMap<FeatureKey, FeatureValue>,
    pub health: HealthAssessment,
    pub provenance_hash: [u8; 32],
}

pub struct MarketSentimentVector {
    pub market_id: MarketId,
    pub horizon: Horizon,
    pub directional_flow: ScoredDimension,
    pub informedness: ScoredDimension,
    pub crowding: ScoredDimension,
    pub consensus_independence: ScoredDimension,
    pub leverage_pressure: ScoredDimension,
    pub liquidation_fragility: ScoredDimension,
    pub liquidity_quality: ScoredDimension,
    pub carry_pressure: ScoredDimension,
    pub positioning_dispersion: ScoredDimension,
    pub regime: RegimeAssessment,
    pub confidence: ProbabilityPpm,
    pub data_freshness: ProbabilityPpm,
    pub as_of_block: BlockHeight,
    pub provenance_hash: [u8; 32],
}
```

`ScoredDimension::try_new` rejects non-finite analytical conversions, unordered intervals, empty health scope, and unsupported unit/value combinations. `MarketFeatureSnapshot` is the sole input type accepted by live and replay signal evaluation.

- [ ] **Step 3: Implement immutable cohort definitions and as-of membership**

Cohort definitions are a versioned expression AST, not arbitrary SQL:

```rust
pub enum RatioMeasure {
    IndependentEntityCount,
    GrossExposure,
    NewRiskFlow,
    HighConvictionFlow,
    LiquidationWeightedExposure,
    TakerOpeningFlow,
    SmartCrowdDivergence,
}

pub enum RatioUnit { Count, Usd, ProbabilityPpm, Dimensionless }

pub struct RatioScope {
    pub numerator_cohort_id: CohortId,
    pub denominator_cohort_id: CohortId,
    pub measure: RatioMeasure,
    pub unit: RatioUnit,
    pub horizon: Horizon,
    pub exclusions: Vec<String>,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub as_of_block: BlockHeight,
}

pub enum CohortPredicate {
    SkillProbabilityAtLeast(ProbabilityPpm),
    StyleProbabilityAtLeast { style: StyleClass, value: ProbabilityPpm },
    IntentProbabilityAtLeast { intent: IntentClass, value: ProbabilityPpm },
    EquityPercentileAtLeast(ProbabilityPpm),
    LeverageAtLeast(Leverage),
    BehaviorRegime(RegimeId),
    And(Vec<CohortPredicate>),
    Or(Vec<CohortPredicate>),
    Not(Box<CohortPredicate>),
}
```

Evaluation uses feature and cluster versions known at the requested time.

- [ ] **Step 4: Implement scoped ratios**

Support exact result types for independent-entity count, cohort gross exposure, new-risk flow, high-conviction flow, liquidation-weighted, taker-opening, and smart-versus-crowd divergence. Every result requires a `RatioScope` and refuses an empty/ambiguous denominator.

- [ ] **Step 5: Verify migrations and property tests**

```bash
cargo test -p market-intelligence ratios
clickhouse-client --multiquery < schemas/clickhouse/0006_market_features.sql
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schemas/postgres/0003_cohort_definitions.sql
```

Expected: exact ratio behavior, stable cohort hashes, and no future membership leakage.

- [ ] **Step 6: Commit**

```bash
git add crates/market-intelligence schemas/proto/intelligence/v1/market.proto schemas/clickhouse/0006_market_features.sql schemas/postgres/0003_cohort_definitions.sql docs/metrics/market-sentiment.md Cargo.toml Cargo.lock
git commit -m "feat(market): define sentiment and scoped cohort ratios"
```

---

### Task 2: Implement Smart Flow, informed aggression, and conviction

**Files:**
- Create: `crates/market-intelligence/src/flow.rs`
- Create: `crates/market-intelligence/src/aggression.rs`
- Create: `crates/market-intelligence/src/conviction.rs`
- Create: `crates/market-intelligence/src/normalization.rs`
- Create: `crates/market-intelligence/tests/flow.rs`
- Create: `config/features/market-flow-v1.toml`
- Create: `docs/metrics/smart-flow.md`

**Interfaces:**
- Consumes: material account/entity actions, intent, skill, expected edge after cost, regime fit, copyability, independence, freshness, book depth, volume, OI, and health.
- Produces: raw and normalized Smart Flow, informed taker aggression, conviction components, and component-level evidence.

- [ ] **Step 1: Write opening-versus-closing and independence tests**

Assert opening a long contributes positive directional new risk; closing a short is a buy but remains a distinct close-risk component; reducing a long contributes negative risk; a static position contributes zero. Twenty linked/follower addresses contribute approximately one independent vote.

- [ ] **Step 2: Implement exact weighted flow accumulation**

```rust
pub struct SmartFlowContribution {
    pub subject: EntityId,
    pub signed_new_risk_usd: UsdAmount,
    pub skill_probability: ProbabilityPpm,
    pub expected_edge_after_cost_bps: BasisPoints,
    pub regime_fit: ProbabilityPpm,
    pub copyability: ProbabilityPpm,
    pub independence_weight: ProbabilityPpm,
    pub data_confidence: ProbabilityPpm,
    pub freshness_decay: ProbabilityPpm,
    pub intent_adjustment: ProbabilityPpm,
}
```

Multiply through checked scaled-integer arithmetic with reviewed rounding. Keep each component and final contribution in evidence.

- [ ] **Step 3: Implement liquidity normalization and robust z-scores**

The normalizer combines recent volume, open interest, and executable depth using versioned coefficients and minimum floors. Robust z-scores use historical market/regime distributions known at the time; raw dollar-equivalent flow remains visible.

- [ ] **Step 4: Implement informed taker aggression and conviction**

Aggression separates opening long, closing short, opening short, and closing long. Conviction exposes position/equity delta, leverage change, aggressive spread crossing, preceding capital activation, additions through adverse movement, concentration change, persistence, and visible hedge evidence. Skill and conviction are never merged.

- [ ] **Step 5: Run invariance and sensitivity tests**

```bash
cargo test -p market-intelligence flow
```

Expected: splitting one entity across linked accounts does not materially change aggregate flow; worse health/freshness/copyability never increases weighted flow; static positions remain zero.

- [ ] **Step 6: Commit**

```bash
git add crates/market-intelligence/src/flow.rs crates/market-intelligence/src/aggression.rs crates/market-intelligence/src/conviction.rs crates/market-intelligence/src/normalization.rs crates/market-intelligence/tests/flow.rs config/features/market-flow-v1.toml docs/metrics/smart-flow.md
git commit -m "feat(market): compute smart flow aggression and conviction"
```

---

### Task 3: Implement interpretable market-regime state

**Files:**
- Create: `crates/market-intelligence/src/regime/mod.rs`
- Create: `crates/market-intelligence/src/regime/features.rs`
- Create: `crates/market-intelligence/src/regime/model.rs`
- Create: `crates/market-intelligence/src/regime/names.rs`
- Create: `crates/market-intelligence/tests/regime.rs`
- Create: `config/models/market-regime-v1.toml`
- Create: `docs/models/market-regime-v1.md`
- Create: `fixtures/models/market-regime-v1/uptrend.json`
- Create: `fixtures/models/market-regime-v1/downtrend.json`
- Create: `fixtures/models/market-regime-v1/range.json`
- Create: `fixtures/models/market-regime-v1/high-volatility.json`
- Create: `fixtures/models/market-regime-v1/liquidity-stress.json`
- Create: `fixtures/models/market-regime-v1/ambiguous.json`

**Interfaces:**
- Consumes: trend, realized volatility, liquidity quality, funding/basis, OI, cross-asset correlation stress, and liquidation intensity.
- Produces: stable named regime probabilities, change probability, feature contributions, support status, and temporal regime versions.

- [ ] **Step 1: Write stable-name and no-lookahead tests**

Generate fixtures for quiet range, orderly trend, leveraged trend, liquidity stress, and post-liquidation recovery. Assert future observations do not alter historical regime state and state labels remain stable across retraining/configuration patch versions.

- [ ] **Step 2: Define the eight approved regime names and assessment contract**

```rust
pub enum RegimeName {
    QuietRange,
    VolatileRange,
    OrderlyUptrend,
    OrderlyDowntrend,
    LeveragedUptrend,
    LeveragedDowntrend,
    LiquidityStress,
    PostLiquidationRecovery,
}

pub struct RegimeAssessment {
    pub probabilities: BTreeMap<RegimeName, ProbabilityPpm>,
    pub change_probability: ProbabilityPpm,
    pub calibration: CalibrationStatus,
    pub support: ApplicabilitySupport,
    pub contributions: Vec<(FeatureKey, Decimal)>,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub model_version: ModelVersion,
}
```

Probabilities sum to exactly 1,000,000 ppm after deterministic largest-remainder rounding. Names never change meaning; a changed taxonomy creates a new model major version.

```rust
pub enum MarketRegime {
    QuietRange,
    VolatileRange,
    OrderlyUptrend,
    OrderlyDowntrend,
    LeveragedUptrend,
    LeveragedDowntrend,
    LiquidityStress,
    PostLiquidationRecovery,
}
```

- [ ] **Step 3: Implement the transparent online model**

Use reviewed standardized features plus an online hidden-state or change-point model with explicit transition matrix, emission parameters, minimum dwell, and fallback rules. Configuration is signed/hash-versioned. The output exposes probabilities and top feature contributions.

- [ ] **Step 4: Implement support and uncertainty behavior**

Out-of-distribution feature vectors return reduced confidence and `outside_training_support`; missing required book/OI/funding inputs yields no canonical regime-dependent signal, not an optimistic default.

- [ ] **Step 5: Validate temporal stability and calibration**

```bash
cargo test -p market-intelligence regime
cargo run -p hl-analytics -- evaluate-regime fixtures/models/market-regime-v1
```

Expected: transition/dwell behavior matches fixtures and calibration report is stored in the model card.

- [ ] **Step 6: Commit**

```bash
git add crates/market-intelligence/src/regime crates/market-intelligence/tests/regime.rs config/models/market-regime-v1.toml docs/models/market-regime-v1.md fixtures/models/market-regime-v1
git commit -m "feat(market): classify interpretable market regimes"
```

---

### Task 4: Implement crowding, saturation, entry, and pain maps

**Files:**
- Create: `crates/market-intelligence/src/crowding.rs`
- Create: `crates/market-intelligence/src/entry_map.rs`
- Create: `crates/market-intelligence/src/pain.rs`
- Create: `crates/market-intelligence/tests/crowding.rs`
- Create: `schemas/parquet/market-positioning-v1.json`
- Create: `docs/metrics/crowding-entry-pain.md`

**Interfaces:**
- Consumes: entity positions, entries, break-even after fees/funding, behavior/style/skill cohorts, leader/follower edges, leverage, margin, funding percentile, and book capacity.
- Produces: crowding/saturation dimensions, weighted entry distributions, break-even/underwater/near-liquidation maps, voluntary-exit pressure, and cohort positioning dispersion.

- [ ] **Step 1: Write originator-versus-follower saturation tests**

Create one originator followed by 100 linked/follower accounts. Assert directional consensus can remain positive while saturation rises and remaining capacity falls. Create 10 independent entries at dispersed prices; assert lower entry concentration than one tightly clustered cohort.

- [ ] **Step 2: Implement crowding components**

Expose independent entity count, effective exposure concentration, follower saturation, post-originator flow share, entry clustering, funding percentile, leverage concentration, and observed capacity consumed. Each component has its own interval/health.

- [ ] **Step 3: Implement weighted entry and break-even distributions**

Use analytical episode entry VWAP and current fees/funding to compute break-even. Aggregate by cohort/entity independence with configurable price bins expressed in bps from mark. Preserve raw observations in archive and publish compact histogram/quantile outputs.

- [ ] **Step 4: Implement pain-state classification**

Classify position mass as profitable, near break-even, underwater, voluntary-exit pressure, near-liquidation, or unknown. Thresholds are versioned and account for side, age, normal hold distribution, leverage, and margin uncertainty.

- [ ] **Step 5: Verify conservation and temporal behavior**

```bash
cargo test -p market-intelligence crowding
```

Expected: histogram mass equals eligible scoped position mass within exact rounding; follower-account duplication does not inflate independent count; unknown margin stays unknown.

- [ ] **Step 6: Commit**

```bash
git add crates/market-intelligence/src/crowding.rs crates/market-intelligence/src/entry_map.rs crates/market-intelligence/src/pain.rs crates/market-intelligence/tests/crowding.rs schemas/parquet/market-positioning-v1.json docs/metrics/crowding-entry-pain.md
git commit -m "feat(market): map crowding entries and cohort pain"
```

---

### Task 5: Implement iterative liquidation-fragility simulation

**Files:**
- Create: `crates/market-intelligence/src/fragility/mod.rs`
- Create: `crates/market-intelligence/src/fragility/scenario.rs`
- Create: `crates/market-intelligence/src/fragility/liquidations.rs`
- Create: `crates/market-intelligence/src/fragility/contagion.rs`
- Create: `crates/market-intelligence/src/fragility/bounds.rs`
- Create: `crates/market-intelligence/tests/fragility.rs`
- Create: `config/features/fragility-v1.toml`
- Create: `fixtures/models/fragility-known-episodes/long-cascade.json`
- Create: `fixtures/models/fragility-known-episodes/short-cascade.json`
- Create: `fixtures/models/fragility-known-episodes/portfolio-margin-uncertain.json`
- Create: `fixtures/models/fragility-known-episodes/no-cascade.json`
- Create: `docs/models/liquidation-fragility-v1.md`

**Interfaces:**
- Consumes: exact account/position/collateral state, margin adapters, oracles, L4 books, liquidation rules, entity links, shared collateral, and uncertainty bounds.
- Produces: low/base/high path simulations, first/second-wave forced flow, impact, fragility ratios, vulnerable concentration, cross-market contagion, and confidence.

- [ ] **Step 1: Write deterministic cascade tests**

Create a small market where a -1% shock liquidates account A, its book impact causes account B to cross maintenance, and the second wave stops. Assert iteration count, orders, price path, and forced notional. Run the same scenario twice and compare full output hash.

- [ ] **Step 2: Define scenario and result contracts**

```rust
pub struct FragilityScenario {
    pub scenario_id: ScenarioId,
    pub shocks_bps: Vec<BasisPoints>,
    pub max_iterations: u32,
    pub max_total_impact_bps: BasisPoints,
    pub liquidation_participation: ProbabilityPpm,
    pub book_stress_multiplier: ProbabilityPpm,
}

pub struct LiquidationWave {
    pub iteration: u32,
    pub liquidated_accounts: Vec<AccountId>,
    pub forced_notional: UsdAmount,
    pub estimated_impact_bps: BasisPoints,
}

pub struct ScenarioPathResult {
    pub terminal_price_change_bps: BasisPoints,
    pub waves: Vec<LiquidationWave>,
    pub total_forced_notional: UsdAmount,
    pub absorbed_notional: UsdAmount,
    pub vulnerable_notional_remaining: UsdAmount,
    pub iteration_limit_reached: bool,
    pub health: HealthAssessment,
}

pub struct FragilityResult {
    pub low: ScenarioPathResult,
    pub base: ScenarioPathResult,
    pub high: ScenarioPathResult,
    pub confidence: ProbabilityPpm,
    pub missing_inputs: Vec<String>,
    pub provenance_hash: [u8; 32],
}
```

Default shocks are the approved ±0.25%, ±0.50%, ±1%, ±2%, ±3%, and ±5% grid.

- [ ] **Step 3: Implement iterative repricing and liquidation order generation**

At each step: reprice marks/collateral/oracles, evaluate every affected account with the correct margin model, generate expected liquidation orders, walk them through the book, update prices/account state, detect newly vulnerable accounts, and stop at fixed point or scenario limit. Never mutate canonical live state; simulation uses a copy-on-write scenario state.

- [ ] **Step 4: Implement uncertainty and contagion bounds**

Portfolio-margin and external-hedge uncertainty create low/base/high variants. Shared accounts/collateral propagate across markets. Red book or unsupported margin mode yields red result for affected path and suppresses fragility signals.

- [ ] **Step 5: Validate against known liquidation episodes and performance budgets**

```bash
cargo test -p market-intelligence fragility
cargo run -p hl-analytics -- evaluate-fragility fixtures/models/fragility-known-episodes
cargo bench -p market-intelligence --bench fragility
```

Document error bands for first-wave notional, impact, side asymmetry, and vulnerable concentration. Benchmark common scenarios to fit live update budgets.

- [ ] **Step 6: Commit**

```bash
git add crates/market-intelligence/src/fragility crates/market-intelligence/tests/fragility.rs config/features/fragility-v1.toml fixtures/models/fragility-known-episodes docs/models/liquidation-fragility-v1.md
git commit -m "feat(market): simulate iterative liquidation fragility"
```

---

### Task 6: Implement market memory and cross-asset intelligence

**Files:**
- Create: `crates/market-intelligence/src/memory/mod.rs`
- Create: `crates/market-intelligence/src/memory/vector.rs`
- Create: `crates/market-intelligence/src/memory/index.rs`
- Create: `crates/market-intelligence/src/cross_asset.rs`
- Create: `crates/market-intelligence/tests/memory.rs`
- Create: `crates/storage-ports/src/vector_index.rs`
- Create: `services/hl-analytics/src/market_memory.rs`
- Create: `schemas/clickhouse/0007_market_memory.sql`
- Create: `docs/metrics/market-memory.md`

**Interfaces:**
- Consumes: standardized point-in-time market vectors, independent episode boundaries, executable outcome summaries, entity rotation, shared collateral, correlations, and lead/lag features.
- Produces: historical analogue sets with dimension attribution/support flags and cross-asset risk/rotation features.

- [ ] **Step 1: Write no-self-match and episode-decorrelation tests**

A query cannot match the current episode or overlapping future window. Ten adjacent snapshots from one episode count as one independent episode in outcome summaries. An outside-support query must be labeled rather than forced to a misleading nearest match.

- [ ] **Step 2: Define a versioned standardized vector**

Vector dimensions include the sentiment components, regime probabilities, liquidity/OI/funding, crowding, fragility, and selected cross-asset context. Store dimension names/order/scaling in a feature-set manifest; index artifacts include the manifest hash.

- [ ] **Step 3: Implement exact baseline then optional approximate index**

Start with exact distance search through a `VectorIndex` port for validation. Add HNSW only after recall/latency benchmarks. Approximate search results are reranked exactly and must meet documented recall against the exact baseline.

- [ ] **Step 4: Implement analogue output and cross-asset features**

Return similarity, contributing dimensions, independent episode count, executable outcome distribution, regime/liquidity differences, and support status. Cross-asset outputs cover entity rotation, simultaneous deleveraging, lead/lag, shared collateral contagion, beta-neutral positioning, correlation stress, and entity gross/net risk.

- [ ] **Step 5: Verify deterministic indexing and recall**

```bash
cargo test -p market-intelligence memory
cargo run -p hl-analytics -- build-market-memory --manifest tests/regression/market-memory/manifest.toml
cargo run -p hl-analytics -- verify-market-memory --exact-sample 1000
```

Expected: repeat builds yield identical vector/provenance hashes; approximate recall exceeds the approved threshold or exact search remains the production path.

- [ ] **Step 6: Commit**

```bash
git add crates/market-intelligence/src/memory crates/market-intelligence/src/cross_asset.rs crates/market-intelligence/tests/memory.rs crates/storage-ports/src/vector_index.rs services/hl-analytics/src/market_memory.rs schemas/clickhouse/0007_market_memory.sql docs/metrics/market-memory.md
git commit -m "feat(market): retrieve point-in-time analogues and cross-asset context"
```

---

### Task 7: Implement signal objects, evidence, lifecycle, and invalidation

**Files:**
- Modify: `crates/signal-core/src/lib.rs`
- Create: `crates/signal-core/src/signal.rs`
- Create: `crates/signal-core/src/evidence.rs`
- Create: `crates/signal-core/src/lifecycle.rs`
- Create: `crates/signal-core/src/invalidation.rs`
- Create: `crates/signal-core/src/errors.rs`
- Create: `crates/signal-core/tests/lifecycle.rs`
- Create: `schemas/proto/signal/v1/signal.proto`
- Create: `schemas/clickhouse/0008_signals.sql`
- Create: `docs/contracts/signal-v1.md`

**Interfaces:**
- Consumes: market/entity/wallet features, health, execution/capacity estimates, model/build versions, historical analogue references.
- Produces: immutable `Signal`, append-only lifecycle events, complete evidence bundles, deterministic invalidation evaluation, and outcome hooks.

- [ ] **Step 1: Write lifecycle and evidence-completeness tests**

Assert invalid transitions such as `Candidate -> Live` fail. Assert a candidate missing cost assumptions, model hash, data watermark, or invalidation rule cannot become `Validated` or `Live`. Assert a red required dependency immediately invalidates or suppresses according to policy.

- [ ] **Step 2: Define the exact signal contract**

```rust
pub enum SignalType {
    IndependentSmartFlowAcceleration,
    SmartCrowdDivergence,
    LiquidationFragilityAsymmetry,
    ResearchOnly(String),
}

pub enum SignalLifecycleState {
    Candidate,
    Validated,
    Live,
    Decaying,
    Invalidated,
    Expired,
    Resolved,
}

pub struct Signal {
    pub signal_id: SignalId,
    pub signal_type: SignalType,
    pub market_id: MarketId,
    pub direction: Direction,
    pub created_at: KnownTime,
    pub effective_at: ProtocolTime,
    pub as_of_block: BlockHeight,
    pub confirmation_class: ConfirmationClass,
    pub horizon: Horizon,
    pub expected_return_bps: BasisPoints,
    pub expected_cost_bps: BasisPoints,
    pub net_edge_bps: BasisPoints,
    pub confidence: ProbabilityPpm,
    pub confidence_interval_bps: ClosedInterval<BasisPoints>,
    pub capacity: UsdAmount,
    pub half_life: Horizon,
    pub crowding: ProbabilityPpm,
    pub tail_risk_bps: BasisPoints,
    pub data_health: HealthAssessment,
    pub model_version: ModelVersion,
    pub feature_set_version: FeatureSetVersion,
    pub evidence_bundle_hash: [u8; 32],
    pub invalidation_rules: Vec<InvalidationRule>,
    pub lifecycle_state: SignalLifecycleState,
}

pub enum SignalActor { System, ResearchRole, RiskRole, PlatformRole }

pub struct SignalLifecycleEvent {
    pub signal_id: SignalId,
    pub previous: Option<SignalLifecycleState>,
    pub next: SignalLifecycleState,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub reason_code: String,
    pub evidence_bundle_hash: [u8; 32],
    pub build_commit: String,
    pub model_version: ModelVersion,
    pub feature_set_version: FeatureSetVersion,
    pub actor: SignalActor,
}

pub enum SignalError {
    IncompleteEvidence(Vec<String>),
    InvalidTransition { from: SignalLifecycleState, to: SignalLifecycleState },
    UnsupportedHealth(HealthAssessment),
    ContractViolation(String),
}
```

`ResearchOnly` identifiers must be registered and cannot transition to `Live`. Lifecycle events are append-only; current state is a fold over events.

- [ ] **Step 3: Implement evidence bundles as content-addressed objects**

Evidence includes canonical event refs, wallets/entities/weights, feature before/after, watermarks/source confidence, model artifact and code commit, cost assumptions, analogues, invalidation rules, capacity, half-life, and limitations. Serialize canonically and hash; the hash is immutable.

- [ ] **Step 4: Implement typed invalidation rules**

```rust
pub enum InvalidationRule {
    OriginatorExposureClosed { fraction: ProbabilityPpm },
    FlowBelow { feature: FeatureKey, threshold: FeatureValue },
    IndependenceBelow(ProbabilityPpm),
    CostAboveEdge,
    DataHealthNotGreen,
    BookHealthNotGreen,
    TimeExpired { at: ProtocolTime },
    CustomApproved { rule_id: String, version: u32 },
}
```

Rules are deterministic, versioned, and show current distance-to-invalidation.

- [ ] **Step 5: Verify serialization and append-only persistence**

```bash
cargo test -p signal-core lifecycle
cargo run -p schema-check -- check schemas/proto/baseline/v1.pb target/schema/current.pb
```

Expected: stable hashes and no mutable signal-state row updates; lifecycle is append-only.

- [ ] **Step 6: Commit**

```bash
git add crates/signal-core schemas/proto/signal schemas/clickhouse/0008_signals.sql docs/contracts/signal-v1.md Cargo.toml Cargo.lock
git commit -m "feat(signal): add evidence-complete signal lifecycle"
```

---

### Task 8: Implement the three V1 signal families

**Files:**
- Create: `crates/signal-core/src/families/mod.rs`
- Create: `crates/signal-core/src/families/smart_flow_acceleration.rs`
- Create: `crates/signal-core/src/families/smart_crowd_divergence.rs`
- Create: `crates/signal-core/src/families/fragility_asymmetry.rs`
- Create: `crates/signal-core/tests/families.rs`
- Create: `config/signals/v1/independent-smart-flow.toml`
- Create: `config/signals/v1/smart-crowd-divergence.toml`
- Create: `config/signals/v1/liquidation-fragility-asymmetry.toml`
- Create: `docs/signals/independent-smart-flow.md`
- Create: `docs/signals/smart-crowd-divergence.md`
- Create: `docs/signals/liquidation-fragility-asymmetry.md`

**Interfaces:**
- Consumes: current market feature snapshots, wallet/entity intelligence, execution costs/capacity, health, regime, crowding, fragility, and historical evidence.
- Produces: candidate decisions and explicit no-signal/suppression decisions for exactly three production signal types.

- [ ] **Step 1: Write positive, negative, and suppression fixtures for each family**

Each family gets fixtures for trigger, just-below threshold, data red, book red, follower-dominated flow, insufficient independent entities, cost consumes edge, and invalidation after trigger.

- [ ] **Step 2: Implement a shared evaluator contract**

```rust
pub struct SignalContext {
    pub wallet_intelligence: Vec<WalletIntelligenceVector>,
    pub independence_weights: BTreeMap<EntityId, ProbabilityPpm>,
    pub execution_cost_bps: BasisPoints,
    pub executable_capacity: UsdAmount,
    pub regime: RegimeAssessment,
    pub crowding: ScoredDimension,
    pub fragility: FragilityResult,
    pub historical_support: ProbabilityPpm,
    pub required_health: HealthAssessment,
}

pub trait SignalEvaluator {
    fn signal_type(&self) -> SignalType;
    fn evaluate(
        &self,
        snapshot: &MarketFeatureSnapshot,
        context: &SignalContext,
    ) -> Result<SignalEvaluation, SignalError>;
}

pub enum SignalEvaluation {
    Candidate(Signal),
    NoSignal { reasons: Vec<String> },
    Suppressed { health: HealthAssessment, reasons: Vec<String> },
}
```

- [ ] **Step 3: Implement independent smart-flow acceleration**

Require multiple independent currently relevant skilled entities, positive signed new-risk acceleration, positive historical markout support, sufficient capacity, and non-consumed crowding. Invalidate on originator close, flow reversal, independence drop, cost above edge, or health degradation.

- [ ] **Step 4: Implement smart-versus-crowd divergence**

Require explicit smart and crowd cohort definitions, opposite new-risk flow, minimum effective sample sizes, regime/liquidity context, and evidence that market-maker/carry/follower intent does not explain the smart side. Invalidate when divergence closes without response, smart side becomes follower-dominated, or intent explanation changes.

- [ ] **Step 5: Implement liquidation-fragility asymmetry**

Require low/base/high scenario outputs, side asymmetry beyond policy, insufficient book absorption, and direction logic conditioned on trigger distance/current flow. Red/unsupported margin or book state suppresses the family.

- [ ] **Step 6: Run exact regression tests and commit**

```bash
cargo test -p signal-core families
```

Expected: only the three V1 types can produce production candidates; other registered hypotheses remain `ResearchOnly` and cannot transition to live.

```bash
git add crates/signal-core/src/families crates/signal-core/tests/families.rs config/signals/v1 docs/signals
git commit -m "feat(signal): add three V1 alpha signal families"
```

---

### Task 9: Implement utility, deduplication, fatigue controls, and live materialization

**Files:**
- Create: `crates/signal-core/src/utility.rs`
- Create: `crates/signal-core/src/dedup.rs`
- Create: `crates/signal-core/src/alert_policy.rs`
- Create: `crates/signal-core/tests/dedup.rs`
- Create: `services/hl-core/src/features.rs`
- Create: `services/hl-core/src/signals.rs`
- Create: `services/hl-analytics/src/market.rs`
- Create: `services/hl-analytics/src/signals.rs`
- Create: `services/hl-analytics/tests/market_signal_equivalence.rs`
- Create: `infra/monitoring/dashboards/market-signals.json`
- Create: `infra/monitoring/alerts/market-signals.yml`

**Interfaces:**
- Consumes: feature updates, signal evaluators, portfolio-neutral canonical utility inputs, health, and persistence/bus ports.
- Produces: canonical signal utility, evolving signal threads, material-change updates, cooldown/fatigue decisions, live/replay-equivalent snapshots, and signal streams.

- [ ] **Step 1: Write deduplication and invalidation-priority tests**

Repeated evidence within thresholds updates one signal thread, not new alerts. Cooldown suppresses non-material updates but never suppresses invalidation or risk escalation. Independent evidence sets may create separate threads.

- [ ] **Step 2: Implement canonical utility without personal portfolio state**

Canonical utility combines net expected return, calibrated confidence, generic capacity fit, freshness, tail risk, crowding, and data uncertainty. Personal portfolio correlation/constraints are applied by the desk/API plan and never alter the canonical signal.

- [ ] **Step 3: Implement one evolving thread per market/family/evidence origin**

A deduplication key uses market, family, originator entity set hash, direction, and evidence independence class. Material-change thresholds are versioned by family. All updates append lifecycle/evidence records.

- [ ] **Step 4: Integrate online market features and signals**

`hl-core` updates bounded online market features after the state block barrier, evaluates deterministic signal components, persists/publishes after health checks, and records explicit suppression metrics. `hl-analytics` stores history and computes heavier analogue/batch corrections through the same feature/evaluator interfaces.

- [ ] **Step 5: Verify online/replay equivalence and latency**

```bash
just dev-up
cargo test -p hl-analytics --test market_signal_equivalence
cargo bench -p signal-core --bench evaluation
just dev-down
```

Expected: feature/signal/evidence hashes match replay; feature update to deterministic decision fits p99 50 ms target on target hardware.

- [ ] **Step 6: Commit**

```bash
git add crates/signal-core/src/utility.rs crates/signal-core/src/dedup.rs crates/signal-core/src/alert_policy.rs crates/signal-core/tests/dedup.rs services/hl-core/src/features.rs services/hl-core/src/signals.rs services/hl-analytics/src/market.rs services/hl-analytics/src/signals.rs services/hl-analytics/tests/market_signal_equivalence.rs infra/monitoring/dashboards/market-signals.json infra/monitoring/alerts/market-signals.yml
git commit -m "feat(signals): rank deduplicate and materialize live signals"
```

---

### Task 10: Execute the Stage 4 market-intelligence gate

**Files:**
- Create or modify before verification: `config/stage-gates/stage-4.toml`
- Create before verification: `tests/regression/market-intelligence/manifest.toml`
- Create or modify before verification: `docs/reviews/market-metrics-behavior-v1.md`
- Create or modify before verification: `docs/reviews/fragility-episode-validation-v1.md`
- Create or modify before verification: `justfile`
- Generate after verification: `docs/stage-gates/stage-4-market-intelligence.evidence.json`
- Generate after verification: `docs/stage-gates/stage-4-market-intelligence.md`

**Interfaces:**
- Consumes: the complete stage implementation, approved point-in-time regression material, and prior signed gate evidence.
- Produces: a clean-commit canonical gate report, signed approval record, and signed `stage-4-market` tag.

- [ ] **Step 1: Freeze the regression and review inputs**

Freeze varied markets, regimes, liquidity conditions, follower-saturated flow, independent smart flow, carry and market-maker activity, leverage extremes, known liquidation cascades, and unsupported or uncertain margin cases. Every case records temporal cutoffs and expected health behavior.

- [ ] **Step 2: Implement the exact gate configuration and tests**

`just stage-4-gate` writes only to ignored `target/stage-gates/stage-4.json` and runs ratio semantics; flow independence; regime no-lookahead; crowding conservation; deterministic and known-episode fragility validation; market-memory no-self-match and recall; signal lifecycle/evidence checks; every family trigger/suppression/invalidation fixture; online/replay equality; and latency benchmarks.

The gate runner must reject a dirty worktree before any check, record the clean implementation SHA, and fail closed on missing evidence or approvals. Add a configuration test proving every required command and artifact is present.

- [ ] **Step 3: Commit every gate input before verification**

```bash
git add config/stage-gates/stage-4.toml tests/regression/market-intelligence docs/reviews/market-metrics-behavior-v1.md docs/reviews/fragility-episode-validation-v1.md justfile
git commit -m "chore(gate): add Stage 4 market intelligence verification inputs"
test -z "$(git status --porcelain)"
git rev-parse HEAD
```

The printed SHA is the immutable implementation commit evaluated by this gate.

- [ ] **Step 4: Run the gate from fresh clean clones on two supported hosts**

```bash
just stage-4-gate
cargo run -p market-behavior-report -- tests/regression/market-intelligence/manifest.toml --output target/stage-gates/stage-4-behavior.json
sha256sum target/stage-gates/stage-4.json
```

Expected: PASS; canonical report, state/output hashes, and configured reproducibility views agree across hosts. Host-specific provenance remains recorded but is excluded only from the explicitly defined cross-host comparison projection.

- [ ] **Step 5: Commit evidence, collect approvals, and sign the stage tag**

```bash
cp target/stage-gates/stage-4.json docs/stage-gates/stage-4-market-intelligence.evidence.json
cargo run -p stage-gate -- render-record --evidence docs/stage-gates/stage-4-market-intelligence.evidence.json --output docs/stage-gates/stage-4-market-intelligence.md
git add docs/stage-gates/stage-4-market-intelligence.evidence.json docs/stage-gates/stage-4-market-intelligence.md
git commit -m "docs(gate): record Stage 4 market intelligence evidence"
git tag -s stage-4-market -m "Stage 4 market intelligence gate passed"
git verify-tag stage-4-market
```

Platform/data, research, risk, and independent reviewers must provide the detached approval artifacts referenced by the record. Do not create the tag when a required check, comparison, review, or bounded-limitation statement is missing.

## Stage 4 Exit Criteria

- Every metric has a formal definition, cohort/denominator contract, health policy, tests, and historical behavior report.
- Smart Flow counts fresh risk and applies skill, intent, independence, liquidity, cost, freshness, and health correctly.
- Regime, crowding, entry/pain, fragility, memory, and cross-asset outputs are point-in-time and uncertainty-aware.
- Fragility scenarios reconcile with known episodes within approved error bands or explicitly report unsupported uncertainty.
- Every signal transition is append-only and every live-capable candidate has a complete evidence bundle.
- Only three V1 signal families can enter production state.
- `stage-4-market` is approved and tagged.
