# Stage 5 Alpha Laboratory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a reproducible point-in-time research and validation system that replays canonical history, simulates executable outcomes, controls holdout leakage and multiple testing, governs signed models, and records shadow-live evidence without claiming guaranteed profitability.

**Architecture:** `replay-engine` drives the same reducers, feature calculators, and signal evaluators used live. `execution-sim` models signal latency, book arrival, order types, partial fills, fees, funding, impact, exits, portfolio constraints, and failures. `hl-research` executes immutable experiment manifests and produces content-addressed reports. `model-runtime` validates signed bundles and isolates local ONNX inference; PostgreSQL stores registry/approvals and Parquet stores reproducible artifacts.

**Tech Stack:** Rust 1.97.1, DataFusion, Polars lazy API, Arrow/Parquet, ClickHouse 26.3 LTS, PostgreSQL 18.4, native Rust statistics/bootstrap/multiple-testing tools, ONNX Runtime behind a local worker boundary, Ed25519 signatures, Proptest, deterministic seeded simulation, no hosted model or data service.

## Global Constraints

- Stage 4 tag `stage-4-market` and its gate record must verify before this plan begins.
- A research result cannot promote without a complete immutable experiment manifest.
- Feature, score, cluster, metadata, and model inputs use point-in-time/as-of semantics based on what was known then.
- The primary outcome is executable net return after fees, funding, spread, slippage, impact, partial fills, and latency.
- Promotion results come from the event-driven simulator; vectorized exploration is non-promotable evidence.
- The locked holdout is inaccessible to feature selection/tuning workflows and opening it is an audited state transition.
- All attempted variants are registered for multiple-testing diagnostics.
- Learned models must beat registered transparent baselines out of sample to justify complexity.
- Production artifacts are signed, schema-matched, locally served, resource-bounded, and explicitly approved.
- No self-updating production model or silent retraining/deployment is allowed.
- Shadow live uses actual production arrival/processing latency and no capital.
- Promotion thresholds are versioned policy; loosening them requires justification and a new locked holdout.
- V1 remains read-only.
- Every task follows TDD and ends in a focused commit.

---

### Task 1: Implement immutable experiment registration and holdout access control

**Files:**
- Create: `crates/replay-engine/src/experiment.rs`
- Create: `services/hl-research/src/manifest.rs`
- Create: `services/hl-research/src/registry.rs`
- Create: `services/hl-research/src/hypothesis.rs`
- Create: `services/hl-research/tests/manifest.rs`
- Create: `schemas/postgres/0004_experiments.sql`
- Create: `schemas/proto/research/v1/experiment.proto`
- Create: `docs/research/experiment-governance.md`
- Create: `config/research/example-experiment.toml`

**Interfaces:**
- Consumes: design-required manifest fields, role identities, source data manifests, feature/model versions, and date/block ranges.
- Produces: content-addressed `ExperimentManifest`, immutable hypothesis registration, holdout lock/open events, reviewer records, and experiment IDs.

- [ ] **Step 1: Verify Stage 4 and write manifest-completeness tests**

```bash
git verify-tag stage-4-market
just stage-4-gate
```

Write tests that omit each required field in turn and assert the manifest remains `Exploratory` and cannot transition to `Registered`. Assert changing a registered manifest creates a new experiment ID rather than mutation.

- [ ] **Step 2: Define the exact experiment manifest**

```rust
pub struct ExperimentManifest {
    pub experiment_id: ExperimentId,
    pub hypothesis: String,
    pub owner: String,
    pub code_commit: String,
    pub rust_toolchain: String,
    pub feature_set_version: FeatureSetVersion,
    pub label_definition: LabelDefinitionId,
    pub market_universe_version: String,
    pub wallet_score_version: String,
    pub cluster_version_policy: String,
    pub training_range: BlockRange,
    pub validation_ranges: Vec<BlockRange>,
    pub holdout_range: BlockRange,
    pub data_manifest_hash: [u8; 32],
    pub model_config_hash: [u8; 32],
    pub random_seed: u64,
    pub cost_model_version: String,
    pub execution_latency_assumptions: LatencyAssumptions,
    pub promotion_metrics: Vec<String>,
    pub reviewers: Vec<String>,
}
```

Canonical serialization and hash determine `ExperimentId`; free-form result paths are stored in separate append-only artifact records.

- [ ] **Step 3: Implement holdout states and access policy**

```rust
pub enum HoldoutState { Sealed, OpenedForEvaluation, Closed, Invalidated }
```

Only the evaluation worker account can read sealed holdout partitions, and only after research plus risk approval opens the registered experiment. Every read is audited. A tuning run that references holdout data invalidates the experiment.

- [ ] **Step 4: Implement variant and reviewer tracking**

Every explored parameter/feature/model variant is a child record linked to the hypothesis family. The system records accepted/rejected status and metric summaries so false-discovery and deflated performance diagnostics include failed attempts.

- [ ] **Step 5: Verify database constraints**

```bash
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schemas/postgres/0004_experiments.sql
cargo test -p hl-research manifest
```

Expected: registered manifests are immutable, holdout events are append-only, and incomplete manifests cannot promote.

- [ ] **Step 6: Commit**

```bash
git add crates/replay-engine/src/experiment.rs services/hl-research/src/manifest.rs services/hl-research/src/registry.rs services/hl-research/src/hypothesis.rs services/hl-research/tests/manifest.rs schemas/postgres/0004_experiments.sql schemas/proto/research/v1/experiment.proto docs/research/experiment-governance.md config/research/example-experiment.toml
git commit -m "feat(research): register immutable experiments and holdouts"
```

---

### Task 2: Implement deterministic archive and checkpoint replay

**Files:**
- Modify: `crates/replay-engine/src/lib.rs`
- Create: `crates/replay-engine/src/clock.rs`
- Create: `crates/replay-engine/src/source.rs`
- Create: `crates/replay-engine/src/runner.rs`
- Create: `crates/replay-engine/src/checkpoint.rs`
- Create: `crates/replay-engine/src/failures.rs`
- Create: `crates/replay-engine/tests/equivalence.rs`
- Create: `services/hl-research/src/replay.rs`
- Create: `tools/replay-cli/src/main.rs`
- Create: `docs/research/replay.md`

**Interfaces:**
- Consumes: immutable archive manifests, compatible checkpoints, canonical reducers, feature calculators, signal evaluators, explicit replay clock/seed, and failure schedule.
- Produces: deterministic state/features/signals/outcomes, replay cursors, progress events, and content hashes equal to live processing for identical inputs.

- [ ] **Step 1: Write live-versus-replay equivalence tests**

Feed one regression range through a live-style in-memory bus consumer and through archive replay. Assert identical state checkpoint, wallet/entity/market feature, signal lifecycle, evidence, and suppression hashes.

- [ ] **Step 2: Implement a virtual event-time clock**

```rust
pub trait ReplayClock {
    fn protocol_time(&self) -> ProtocolTime;
    fn known_time(&self) -> KnownTime;
    fn processing_time(&self) -> Duration;
    fn advance_to(&mut self, event: &CanonicalEventEnvelope, simulated_latency: Duration);
}
```

No replay component may call system time. Latency is explicit and seeded where stochastic.

- [ ] **Step 3: Implement archive range and checkpoint selection**

The runner verifies manifest chain, chooses the nearest compatible checkpoint, verifies its state hash, then applies subsequent blocks in exact order. Incompatible schema/feature/model versions fail with a reproducible reason rather than implicit migration.

- [ ] **Step 4: Implement controlled failure injection**

Failure schedules can mark source gap, stale state, book mismatch, model failure, publication delay, and simulated execution rejection at explicit blocks. These produce the same health/suppression paths as production.

- [ ] **Step 5: Add deterministic replay CLI and verify**

```bash
cargo run -p replay-cli -- run \
  --manifest tests/regression/market-intelligence/manifest.toml \
  --from-checkpoint auto \
  --seed 42 \
  --output target/replay-result
sha256sum target/replay-result/summary.json
```

Run twice and on a second clean machine; expected hash is identical.

- [ ] **Step 6: Commit**

```bash
git add crates/replay-engine services/hl-research/src/replay.rs tools/replay-cli docs/research/replay.md Cargo.toml Cargo.lock
git commit -m "feat(replay): add deterministic archive and checkpoint replay"
```

---

### Task 3: Implement exact point-in-time feature/label joins

**Files:**
- Create: `crates/replay-engine/src/asof_join.rs`
- Create: `crates/replay-engine/src/feature_store.rs`
- Create: `crates/replay-engine/src/label_visibility.rs`
- Create: `crates/replay-engine/tests/no_leakage.rs`
- Create: `services/hl-research/src/dataset.rs`
- Create: `schemas/parquet/research-dataset-v1.json`
- Create: `tools/point-in-time-audit/src/main.rs`
- Create: `docs/research/point-in-time-data.md`

**Interfaces:**
- Consumes: bitemporal features, cluster versions, wallet scores, market metadata, corrections, label observation time, and experiment ranges.
- Produces: immutable research datasets where every row includes effective/known time, input version/hash, and a proof that no future information entered.

- [ ] **Step 1: Write adversarial leakage tests**

Create corrections learned after evaluation, wallet outcomes whose labels finish after the feature timestamp, cluster merges discovered later, and metadata changes. Assert none enter the earlier row. Add a deliberately faulty latest-value join fixture and assert the audit tool detects it.

- [ ] **Step 2: Implement typed as-of join policy**

```rust
pub struct AsOfPolicy {
    pub evaluation_effective_at: ProtocolTime,
    pub evaluation_known_at: KnownTime,
    pub label_cutoff: KnownTime,
    pub cluster_policy_version: String,
    pub correction_policy: CorrectionPolicy,
}
```

The join requires both `effective_at <= evaluation_effective_at` and `known_at <= evaluation_known_at`, then selects the highest non-superseded revision known at that time.

- [ ] **Step 3: Implement label visibility**

A forward-horizon label becomes available only after its exit/horizon plus required data completeness and cost inputs are known. Wallet skill used at time `t` excludes labels not yet observable at `t`.

- [ ] **Step 4: Write dataset provenance**

Every output partition records experiment ID, source/feature/cluster/metadata manifests, as-of policy, row count, min/max times, schema fingerprint, code/build ID, and content hash. A dataset with missing provenance cannot be consumed by the evaluation runner.

- [ ] **Step 5: Run point-in-time audit**

```bash
cargo test -p replay-engine --test no_leakage
cargo run -p point-in-time-audit -- verify target/research-dataset
```

Expected: valid dataset prints `point-in-time-audit:pass`; faulty fixture is rejected with exact row/input explanation.

- [ ] **Step 6: Commit**

```bash
git add crates/replay-engine/src/asof_join.rs crates/replay-engine/src/feature_store.rs crates/replay-engine/src/label_visibility.rs crates/replay-engine/tests/no_leakage.rs services/hl-research/src/dataset.rs schemas/parquet/research-dataset-v1.json tools/point-in-time-audit docs/research/point-in-time-data.md
git commit -m "feat(research): build leakage-safe point-in-time datasets"
```

---

### Task 4: Implement the event-driven execution simulator

**Files:**
- Modify: `crates/execution-sim/src/lib.rs`
- Create: `crates/execution-sim/src/order.rs`
- Create: `crates/execution-sim/src/latency.rs`
- Create: `crates/execution-sim/src/fill.rs`
- Create: `crates/execution-sim/src/fees.rs`
- Create: `crates/execution-sim/src/funding.rs`
- Create: `crates/execution-sim/src/impact.rs`
- Create: `crates/execution-sim/src/exit.rs`
- Create: `crates/execution-sim/src/portfolio.rs`
- Create: `crates/execution-sim/src/failure.rs`
- Create: `crates/execution-sim/tests/simulator.rs`
- Create: `fixtures/simulation/market-order-partial-fill.json`
- Create: `fixtures/simulation/ioc-no-fill.json`
- Create: `fixtures/simulation/gtc-queue.json`
- Create: `fixtures/simulation/funding-exit.json`
- Create: `fixtures/simulation/stale-book-reject.json`
- Create: `fixtures/simulation/evidence-invalidation.json`
- Create: `fixtures/simulation/portfolio-contention.json`
- Create: `fixtures/simulation/stressed-exit.json`
- Create: `docs/research/execution-simulator.md`

**Interfaces:**
- Consumes: signal detection event/time, historical L4/L2 books, latency distributions, order policy, fee/funding schedules, bankroll, portfolio constraints, exits, and failure schedules.
- Produces: event-by-event simulated orders/fills, realized and missed opportunity, costs, PnL, exposure, capacity, and traceable execution decisions.

- [ ] **Step 1: Write deterministic order/fill fixtures**

Cover market, IOC, GTC, ALO, partial fill, queue uncertainty, cancel, no fill, book gap, stale data, funding interval, stop, take-profit, time exit, evidence invalidation, simultaneous signals, capital contention, and rejection. Every fixture has an exact event trace and net PnL.

- [ ] **Step 2: Define the simulator state machine**

```rust
pub struct SimulationRequest {
    pub signal: SignalSnapshot,
    pub detection_latency: LatencyDistribution,
    pub order_policy: OrderPolicy,
    pub bankroll: UsdAmount,
    pub participation_limit: ProbabilityPpm,
    pub exit_policy: ExitPolicy,
    pub cost_model_version: String,
    pub seed: u64,
}

pub enum SimulationEvent {
    SignalObserved,
    OrderSubmitted,
    OrderRested,
    PartialFill,
    Fill,
    Cancelled,
    FundingApplied,
    EvidenceInvalidated,
    ExitSubmitted,
    PositionClosed,
    Rejected,
}
```

- [ ] **Step 3: Implement arrival-time book and queue behavior**

Detection and network/processing latency advance the replay clock before order arrival. Market/IOC walks the arrival-time book. GTC/ALO uses observed subsequent order/trade events and a conservative queue-position distribution; unobservable queue priority is represented as uncertainty, not perfect fill.

- [ ] **Step 4: Implement all costs and exits**

Apply exact historical fee/funding schedules, spread, slippage, market impact, partial-fill sizing, exit liquidity under normal/stressed policy, and stop/take-profit/time/evidence exits. Net return uses executable entry/exit VWAP.

- [ ] **Step 5: Implement portfolio contention and failure paths**

Simultaneous signals share bankroll and concentration limits. Failure injection can reject order, delay submission, mark data stale, or remove book liquidity. The trace records every accepted/rejected decision and assumption.

- [ ] **Step 6: Verify against hand calculations and monotonicity**

```bash
cargo test -p execution-sim
cargo bench -p execution-sim --bench event_simulation
```

Expected: hand fixtures match exactly; higher costs/latency/size do not improve outcome in controlled monotone cases; fixed seed reproduces queue samples.

- [ ] **Step 7: Commit**

```bash
git add crates/execution-sim fixtures/simulation docs/research/execution-simulator.md Cargo.toml Cargo.lock
git commit -m "feat(research): add event-driven execution simulator"
```

---

### Task 5: Implement executable labels, metrics, baselines, and statistical diagnostics

**Files:**
- Create: `services/hl-research/src/labels.rs`
- Create: `services/hl-research/src/metrics/mod.rs`
- Create: `services/hl-research/src/metrics/bootstrap.rs`
- Create: `services/hl-research/src/metrics/performance.rs`
- Create: `services/hl-research/src/metrics/calibration.rs`
- Create: `services/hl-research/src/metrics/multiple_testing.rs`
- Create: `services/hl-research/src/baselines.rs`
- Create: `services/hl-research/tests/metrics.rs`
- Create: `docs/research/metrics.md`

**Interfaces:**
- Consumes: simulation traces/outcomes, independent episode IDs, predictions/confidence, attempted variants, benchmark returns, and experiment policy.
- Produces: executable labels, expectancy/distribution/risk/calibration/capacity metrics, bootstrap intervals, deflated performance, multiple-testing diagnostics, and baseline comparisons.

- [ ] **Step 1: Write executable-label tests**

Assert:

```text
net_return = direction × (exit_vwap - entry_vwap) / entry_vwap
             - entry_fees - exit_fees - funding - slippage - impact
```

Partial fills must split realized return and missed opportunity. No-fill is not silently dropped; it contributes to fill rate and opportunity diagnostics.

- [ ] **Step 2: Implement required performance metrics**

Calculate net expectancy, median/tails/downside, top-ranked precision, information coefficient, Sharpe/Sortino/deflated Sharpe diagnostics, maximum drawdown, expected shortfall, hit rate, turnover/holding, fill/missed fill, slippage prediction error, capacity curve, segment performance, signal correlation, portfolio contribution, parameter stability, and revision sensitivity.

- [ ] **Step 3: Implement stationary block bootstrap**

Use seeded stationary block bootstrap over de-correlated episode IDs. Report one-sided lower bounds and block-length sensitivity. A test against a known synthetic distribution verifies interval coverage within tolerance.

- [ ] **Step 4: Implement calibration diagnostics**

Calculate Brier score, reliability error/diagram bins, calibration slope/intercept, and support coverage. An uncalibrated score cannot be serialized as a probability; API/display status must be `uncalibrated`.

- [ ] **Step 5: Implement multiple-testing and baselines**

Track false-discovery diagnostics and deflated performance using the registered variant family. Always evaluate no-trade, momentum/mean-reversion, raw flow, raw whale-size, equal-weight top-wallet, and regime-conditioned linear/logistic baselines.

- [ ] **Step 6: Verify and commit**

```bash
cargo test -p hl-research metrics
```

Expected: metric fixtures and seeded bootstrap are exact/reproducible.

```bash
git add services/hl-research/src/labels.rs services/hl-research/src/metrics services/hl-research/src/baselines.rs services/hl-research/tests/metrics.rs docs/research/metrics.md
git commit -m "feat(research): calculate executable metrics and baselines"
```

---

### Task 6: Implement purged walk-forward, embargo, and locked-holdout evaluation

**Files:**
- Create: `services/hl-research/src/validation/mod.rs`
- Create: `services/hl-research/src/validation/split.rs`
- Create: `services/hl-research/src/validation/purge.rs`
- Create: `services/hl-research/src/validation/runner.rs`
- Create: `services/hl-research/src/validation/holdout.rs`
- Create: `services/hl-research/tests/validation.rs`
- Create: `config/research/validation-policy-v1.toml`
- Create: `docs/research/validation-protocol.md`

**Interfaces:**
- Consumes: registered experiment, point-in-time dataset, label horizon/overlap metadata, model/baseline trainers, simulator, and holdout state.
- Produces: purged walk-forward fold results, embargo proof, locked-holdout result, variant ledger, and immutable evaluation artifacts.

- [ ] **Step 1: Write overlap/purge/embargo tests**

Create labels spanning split boundaries. Assert training rows whose outcomes overlap validation are purged and adjacent rows inside embargo are excluded. Assert holdout bytes cannot be read during discovery/validation.

- [ ] **Step 2: Implement explicit split contracts**

```rust
pub struct ValidationFold {
    pub train: BlockRange,
    pub validation: BlockRange,
    pub purge: Vec<BlockRange>,
    pub embargo: Vec<BlockRange>,
}
```

Folds are generated from registered ranges and maximum label horizon; they are stored in the result manifest.

- [ ] **Step 3: Implement the evaluation runner**

For each fold: fit only approved estimator classes on training, generate predictions on validation, simulate execution event by event, calculate metrics, persist artifacts, and release memory/state before the next fold. The runner records seed, threads, feature order, and runtime versions.

- [ ] **Step 4: Implement one-way holdout opening**

After required approvals, the evaluation worker opens holdout, runs exactly the registered configuration, writes results, and closes access. Any code/config/data hash mismatch aborts. Viewing results then changing the experiment requires a new experiment and holdout.

- [ ] **Step 5: Verify reproducibility**

```bash
cargo test -p hl-research validation
cargo run -p hl-research -- evaluate --experiment fixtures/experiments/baseline-v1.toml
```

Expected: two clean runs produce identical feature, prediction, simulation, and report hashes.

- [ ] **Step 6: Commit**

```bash
git add services/hl-research/src/validation services/hl-research/tests/validation.rs config/research/validation-policy-v1.toml docs/research/validation-protocol.md
git commit -m "feat(research): add purged walk-forward and locked holdout"
```

---

### Task 7: Implement versioned promotion policy and automatic evaluation reports

**Files:**
- Create: `services/hl-research/src/promotion/mod.rs`
- Create: `services/hl-research/src/promotion/policy.rs`
- Create: `services/hl-research/src/promotion/report.rs`
- Create: `services/hl-research/src/promotion/approvals.rs`
- Create: `services/hl-research/tests/promotion.rs`
- Create: `config/research/promotion-policy-v1.toml`
- Create: `schemas/postgres/0005_promotion.sql`
- Create: `docs/research/promotion-policy-v1.md`

**Interfaces:**
- Consumes: registered experiment, validation/holdout/shadow results, data-health proof, capacity, risk budget, baselines, and role approvals.
- Produces: deterministic pass/fail/withheld decisions per gate, signed approval records, and human/machine-readable promotion reports.

- [ ] **Step 1: Encode the approved default policy exactly**

The TOML and typed parser must include:

- at least 100 de-correlated validation+holdout outcomes and 30 holdout outcomes;
- at least 90 calendar days and two materially different regimes unless explicitly event-specific;
- one-sided 95% stationary-block-bootstrap lower net-expectancy bound greater than zero;
- positive expectancy at 1.5x modeled costs;
- positive expectancy at measured production p99 latency plus 250 ms;
- no episode over 20% of total net PnL and no market over 50% unless market-specific;
- drawdown/expected-shortfall inside preregistered budget;
- calibration no worse than registered baseline;
- positive intended-allocation edge at one adverse liquidity decile;
- at least 30 de-correlated shadow outcomes and 30 calendar days, with realized distribution not rejected at 5%;
- reproducibility on two clean machines.

- [ ] **Step 2: Write one failing fixture for every policy rule**

Each fixture violates exactly one rule and must produce a stable reason code such as `PROMOTION_COST_STRESS_FAILED`. Missing evidence produces `WITHHELD`, never pass.

- [ ] **Step 3: Implement role-separated approvals**

Research plus risk approval is mandatory; platform approval is required for code/schema changes. No identity can satisfy conflicting roles for the same artifact when policy requires separation. Approval records include artifact hash and expiry.

- [ ] **Step 4: Generate comprehensive reports**

Report hypothesis, data/manifest hashes, folds, holdout integrity, all attempted variants, costs/latency sensitivities, metrics/intervals, baselines, market/regime breakdowns, capacity, concentration, revision sensitivity, shadow status, limitations, and each gate decision.

- [ ] **Step 5: Verify and commit**

```bash
cargo test -p hl-research promotion
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schemas/postgres/0005_promotion.sql
```

Expected: every threshold has a direct test and report field.

```bash
git add services/hl-research/src/promotion services/hl-research/tests/promotion.rs config/research/promotion-policy-v1.toml schemas/postgres/0005_promotion.sql docs/research/promotion-policy-v1.md
git commit -m "feat(governance): enforce alpha promotion policy"
```

---

### Task 8: Implement signed model bundles and registry state machine

**Files:**
- Modify: `crates/model-runtime/src/lib.rs`
- Create: `crates/model-runtime/src/bundle.rs`
- Create: `crates/model-runtime/src/signature.rs`
- Create: `crates/model-runtime/src/schema.rs`
- Create: `crates/model-runtime/src/registry.rs`
- Create: `crates/model-runtime/tests/bundle.rs`
- Create: `services/hl-research/src/model_registry.rs`
- Create: `schemas/postgres/0006_model_registry.sql`
- Create: `models/approved-public-keys/README.md`
- Create: `models/test-models/linear-v1/model.onnx`
- Create: `models/test-models/linear-v1/manifest.json`
- Create: `models/test-models/linear-v1/features.json`
- Create: `models/test-models/linear-v1/golden-input.json`
- Create: `models/test-models/linear-v1/golden-output.json`
- Create: `models/test-models/linear-v1/signature.ed25519`
- Create: `models/test-models/linear-v1/test-public-key.ed25519`
- Create: `tools/model-inspect/src/main.rs`
- Create: `docs/models/model-bundle.md`

**Interfaces:**
- Consumes: approved evaluation report, ONNX or deterministic-model artifact, feature schema, preprocessing/calibration, data/build manifests, offline signing key, and role approvals.
- Produces: signed immutable model bundles, verified load receipts, revocation list, and controlled registry transitions.

- [ ] **Step 1: Write bundle integrity and transition tests**

Reject missing files, changed feature order, wrong opset/runtime constraint, invalid/unknown signature, expired/revoked model, altered evaluation report, and illegal registry transitions. Assert cached revoked artifacts cannot load.

- [ ] **Step 2: Implement the exact bundle layout**

Require `model.onnx`, `manifest.toml`, `feature-schema.json`, `preprocessing.json`, `calibration.json`, `evaluation.json`, `training-data-manifest.json`, `model-card.md`, and `signature.ed25519`. Deterministic rule models may use an approved artifact file in place of ONNX but retain the same manifest/signature contract.

- [ ] **Step 3: Implement canonical signing and verification**

Hash canonical path/name/content tuples in lexical order; sign with Ed25519 offline key. Production stores only approved public keys and revocation records. The verifier checks semantic model version, feature-set exact match/order, input bounds/missing policy, runtime/opset, review date, use restrictions, and approvers.

- [ ] **Step 4: Implement registry states and approvals**

Enforce `DRAFT -> RESEARCH_PASSED -> HOLDOUT_PASSED -> SHADOW -> APPROVED -> CANARY -> PRODUCTION`, with `DEGRADED`, `RETIRED`, and `REVOKED` transitions as policy permits. State events are append-only and hash-linked.

- [ ] **Step 5: Verify with test keys and model inspection**

```bash
cargo test -p model-runtime bundle
cargo run -p model-inspect -- verify models/test-models/linear-v1
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schemas/postgres/0006_model_registry.sql
```

Expected: valid bundle verifies; one-byte modifications fail.

- [ ] **Step 6: Commit**

```bash
git add crates/model-runtime services/hl-research/src/model_registry.rs schemas/postgres/0006_model_registry.sql models tools/model-inspect docs/models/model-bundle.md Cargo.toml Cargo.lock
git commit -m "feat(models): add signed bundles and governed registry"
```

---

### Task 9: Implement isolated local inference, explanations, shadow-live capture, and decay monitoring

**Files:**
- Create: `crates/model-runtime/src/inference.rs`
- Create: `crates/model-runtime/src/worker.rs`
- Create: `crates/model-runtime/src/explanation.rs`
- Create: `crates/model-runtime/src/drift.rs`
- Create: `crates/model-runtime/tests/inference.rs`
- Create: `services/hl-core/src/model_worker.rs`
- Create: `services/hl-analytics/src/shadow_live.rs`
- Create: `services/hl-analytics/src/model_health.rs`
- Create: `schemas/clickhouse/0009_shadow_model_health.sql`
- Create: `infra/systemd/hl-model-worker@.service`
- Create: `infra/monitoring/dashboards/model-health.json`
- Create: `docs/runbooks/model-degrade-revoke.md`

**Interfaces:**
- Consumes: verified model bundle, feature vector, resource budget, live signals/outcomes/costs, drift policy, and registry state.
- Produces: deterministic predictions/contributions, bounded worker results, shadow-live records, calibration/edge/capacity drift, and automatic degrade/retire recommendations.

- [ ] **Step 1: Write inference safety and fallback tests**

Test wrong dimensions, NaN/non-finite runtime output, input outside bounds, memory/time budget exceedance, worker crash, signature revocation after cache, and feature-schema mismatch. Each case suppresses the learned component or uses an explicitly registered deterministic baseline; it never produces an unverified prediction.

- [ ] **Step 2: Implement worker isolation**

A dedicated local process loads one approved bundle, drops privileges, uses systemd memory/CPU/wall-time limits, accepts fixed Protobuf requests over a Unix socket, validates input, and returns prediction plus contribution vector and model receipt. No network access is required.

- [ ] **Step 3: Implement structured explanations**

For linear/tree models, return top positive/negative contributions, current value versus training distribution, support status, confidence/calibration range, and reviewed counterfactual thresholds. Text generation is optional and non-canonical; raw structured evidence is stored.

- [ ] **Step 4: Implement shadow-live outcome capture**

At production signal time record actual adapter-to-feature-to-decision latency, expected book arrival, prediction, costs/capacity, model/feature/build hashes, and no-capital simulated outcome. Outcome completion is append-only at required horizons.

- [ ] **Step 5: Implement drift and degradation policy**

Monitor feature/prediction distributions, calibration, net edge, capacity, wallet population, regime coverage, and follower saturation. Threshold failure emits a registry proposal to `DEGRADED` or `RETIRED`; no automatic retrain/deploy occurs.

- [ ] **Step 6: Verify and commit**

```bash
cargo test -p model-runtime inference
cargo test -p hl-analytics shadow_live model_health
```

Expected: deterministic fixed-input inference, bounded failures, and reproducible shadow outcome hashes.

```bash
git add crates/model-runtime/src/inference.rs crates/model-runtime/src/worker.rs crates/model-runtime/src/explanation.rs crates/model-runtime/src/drift.rs crates/model-runtime/tests/inference.rs services/hl-core/src/model_worker.rs services/hl-analytics/src/shadow_live.rs services/hl-analytics/src/model_health.rs schemas/clickhouse/0009_shadow_model_health.sql infra/systemd/hl-model-worker@.service infra/monitoring/dashboards/model-health.json docs/runbooks/model-degrade-revoke.md
git commit -m "feat(models): isolate inference and monitor shadow decay"
```

---

### Task 10: Integrate `hl-research` and execute the Stage 5 gate

**Files:**
- Modify before verification: `services/hl-research/src/main.rs`
- Create before verification: `services/hl-research/src/app.rs`
- Create before verification: `services/hl-research/src/report.rs`
- Create before verification: `services/hl-research/tests/end_to_end.rs`
- Create or modify before verification: `config/research.example.toml`
- Create or modify before verification: `config/stage-gates/stage-5.toml`
- Create before verification: `tests/regression/research/manifest.toml`
- Create or modify before verification: `docs/reviews/baseline-experiment-v1.md`
- Create or modify before verification: `justfile`
- Generate after verification: `docs/stage-gates/stage-5-alpha-laboratory.evidence.json`
- Generate after verification: `docs/stage-gates/stage-5-alpha-laboratory.md`

**Interfaces:**
- Consumes: the complete stage implementation, approved point-in-time regression material, and prior signed gate evidence.
- Produces: a clean-commit canonical gate report, signed approval record, and signed `stage-5-research` tag.

- [ ] **Step 1: Freeze the regression and review inputs**

Freeze one transparent baseline experiment before opening its locked holdout: hypothesis, ranges, feature set, signal version, latency distribution, costs, bankrolls, baselines, risk budget, multiple-testing family, and promotion policy. The baseline may remain research-only; the gate evaluates laboratory correctness rather than profitability.

- [ ] **Step 2: Implement the exact gate configuration and tests**

`just stage-5-gate` writes only to ignored `target/stage-gates/stage-5.json` and runs full archive/checkpoint replay and live equivalence; point-in-time leakage audit; simulator fixtures and monotonicity; executable labels, metrics, bootstrap, and calibration; purge/embargo/holdout controls; every promotion-policy failure case; model tamper/revocation checks; isolated-inference resource/fallback checks; two-builder feature/prediction/report reproduction; sensitivity reports; and shadow pipeline smoke tests.

The gate runner must reject a dirty worktree before any check, record the clean implementation SHA, and fail closed on missing evidence or approvals. Add a configuration test proving every required command and artifact is present.

- [ ] **Step 3: Commit every gate input before verification**

```bash
git add services/hl-research config/research.example.toml config/stage-gates/stage-5.toml tests/regression/research docs/reviews/baseline-experiment-v1.md justfile
git commit -m "chore(gate): add Stage 5 alpha laboratory verification inputs"
test -z "$(git status --porcelain)"
git rev-parse HEAD
```

The printed SHA is the immutable implementation commit evaluated by this gate.

- [ ] **Step 4: Run the gate from fresh clean clones on two supported hosts**

```bash
just stage-5-gate
cargo run -p hl-research -- generate-promotion-report --manifest tests/regression/research/manifest.toml --output target/stage-gates/stage-5-promotion.json
sha256sum target/stage-gates/stage-5.json
```

Expected: PASS; canonical report, state/output hashes, and configured reproducibility views agree across hosts. Host-specific provenance remains recorded but is excluded only from the explicitly defined cross-host comparison projection.

- [ ] **Step 5: Commit evidence, collect approvals, and sign the stage tag**

```bash
cp target/stage-gates/stage-5.json docs/stage-gates/stage-5-alpha-laboratory.evidence.json
cargo run -p stage-gate -- render-record --evidence docs/stage-gates/stage-5-alpha-laboratory.evidence.json --output docs/stage-gates/stage-5-alpha-laboratory.md
git add docs/stage-gates/stage-5-alpha-laboratory.evidence.json docs/stage-gates/stage-5-alpha-laboratory.md
git commit -m "docs(gate): record Stage 5 alpha laboratory evidence"
git tag -s stage-5-research -m "Stage 5 alpha laboratory gate passed"
git verify-tag stage-5-research
```

Platform/data, research, risk, and independent reviewers must provide the detached approval artifacts referenced by the record. Do not create the tag when a required check, comparison, review, or bounded-limitation statement is missing.

## Stage 5 Exit Criteria

- One registered baseline experiment reproduces exactly from immutable manifests on two clean machines.
- Point-in-time audits detect and reject future information.
- Promotion outcomes use the event-driven execution simulator with latency, book, partial fills, fees, funding, impact, exits, and constraints.
- Walk-forward, purge, embargo, holdout, multiple-testing, calibration, and sensitivity reports are automatic and test-covered.
- Promotion policy implements every approved default threshold and role gate.
- Model bundles are signed, schema-matched, revocable, and loaded only in a bounded local worker.
- Shadow-live records actual latency/cost/outcome evidence with no capital.
- `stage-5-research` is approved and tagged, without implying guaranteed or realized profitability.
