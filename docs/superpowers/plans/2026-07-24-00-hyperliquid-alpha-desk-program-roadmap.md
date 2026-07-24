# Hyperliquid Alpha Desk Program Roadmap

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the approved private, read-only Hyperliquid Alpha Desk as a sequence of independently testable production increments from deterministic data capture through a native internal analyst desk.

**Architecture:** The program uses a Rust 2024 modular monorepo with five V1 deployables (`hl-capture`, `hl-core`, `hl-analytics`, `hl-research`, and `hl-api`) plus a native SwiftUI macOS/iOS application. Immutable Parquet archives and deterministic replay are the source-of-truth foundation; RocksDB serves exact hot state, ClickHouse serves analytical history, PostgreSQL serves control metadata, and NATS JetStream provides operational fan-out.

**Tech Stack:** Rust 1.97.1, Tokio 1.52.x, Axum 0.8.x, Tonic/Prost 0.14.x, NATS JetStream 2.14.x, RocksDB 11.1.x, ClickHouse 26.3 LTS, PostgreSQL 18.4, Arrow/Parquet, DataFusion, Polars, ONNX Runtime, Swift 6.3, SwiftUI, Swift Charts, GRDB 7.x, Kanidm 1.10.x, Ansible, systemd, Podman, Prometheus/VictoriaMetrics, OpenTelemetry.

## Global Constraints

- The approved source of truth is `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md` at tag `design-approved-v1.0.0`; `spec-v1.0.0` preserves the reviewed design content before approval metadata was recorded.
- Rust production code uses Rust 1.97.1, edition 2024, with committed `Cargo.lock` and no unreviewed `unsafe` blocks.
- Swift code uses Swift 6.3 language mode with strict concurrency and treats controllable concurrency warnings as errors.
- Canonical accounting uses checked fixed-point values. `f64` is forbidden in balances, positions, fees, funding, margin, event identity, and reconciliation.
- Canonical reducers are synchronous and deterministic; asynchronous I/O belongs outside the reducer boundary.
- Every state-affecting message is idempotent by stable `EventId`; transport is at least once and effects are exactly once.
- Live and historical replay use the same parser, reducer, feature, and signal code paths.
- Point-in-time and bitemporal correctness are mandatory. Historical outputs may use only information known at the evaluated timestamp.
- Raw source evidence and canonical events are archived before analytical compaction. ClickHouse is rebuildable and never the only copy.
- V1 is read-only. `hl-exec`, trading credentials, signing keys, order placement, and automatic copy trading are excluded from all V1 artifacts and deployments.
- Canonical signal direction is never produced by a language model. Production learned models must be approved, signed, local, schema-matched artifacts.
- A red data-health dependency suppresses the affected feature or signal. The system fails closed for alpha.
- Every task follows test-driven development, includes exact verification commands, and ends in a focused commit.
- Implementation begins in an isolated worktree created with the `superpowers:using-git-worktrees` workflow.
- Stage gates are evidence-based; no stage advances solely because a date or sprint boundary has arrived.

---

## Plan Set and Dependency Order

| Order | Plan | Stage outcome | Hard dependency |
|---:|---|---|---|
| 1 | `2026-07-24-01-foundations.md` | Reproducible monorepo, schemas, fixtures, CI, local dependencies, telemetry skeleton | Approved design |
| 2 | `2026-07-24-02-truth-layer.md` | Durable source capture, canonical sequencing, gap/divergence handling, immutable archive | Foundations |
| 3 | `2026-07-24-03-state-reconstruction.md` | Deterministic account/market/order-book state, snapshots, reconciliation | Truth layer |
| 4 | `2026-07-24-04-wallet-entity-intelligence.md` | Point-in-time wallet skill, style, intent, copyability, entity and relationship intelligence | State reconstruction |
| 5 | `2026-07-24-05-market-intelligence-signals.md` | Sentiment vector, fragility, market memory, evidence-backed V1 signals | Wallet/entity intelligence and order book |
| 6 | `2026-07-24-06-alpha-laboratory.md` | Replay, execution simulation, experiment governance, promotion gates, signed model bundles | Market/signals and immutable history |
| 7 | `2026-07-24-07-internal-desk.md` | Authenticated API, live streams, macOS desk, iOS companion, shadow portfolio and decision journal | Stable read models and signal contracts |
| 8 | `2026-07-24-08-production-hardening.md` | Production topology, security, observability, recovery, canary and release gate | All V1 stages |

The future execution enclave requires a new threat model, design approval, and implementation plan. It is intentionally absent from this plan set.

## Cross-Stage Interface Freeze Points

1. **After Foundations:** `domain-types`, `canonical-events`, `api-contracts`, error taxonomy, build provenance, and schema compatibility tooling are versioned. Later changes require an explicit compatibility decision.
2. **After Truth Layer:** `SourceObservation`, `BlockEnvelope`, `CanonicalEventEnvelope`, `EventId`, spool segments, archive manifests, and NATS subjects are frozen at semantic major version 1.
3. **After State Reconstruction:** ledger, market registry, position episode, order-book, checkpoint, `StateDelta`, and reconciliation contracts are frozen for downstream feature work.
4. **After Wallet/Entity Intelligence:** point-in-time wallet/entity feature schemas and temporal cluster membership are frozen for market intelligence and research.
5. **After Market/Signals:** signal object, lifecycle, evidence bundle, invalidation rule, feature-set version, and signal outcome contracts are frozen for research and clients.
6. **After Alpha Laboratory:** experiment manifest, promotion report, model bundle, model registry state machine, and shadow-live outcome contracts are frozen for governance.
7. **After Internal Desk:** REST/OpenAPI, stream resume protocol, local cache schema, decision record, and shadow portfolio contracts are frozen for production release.

## Program-Level Quality Gates

- `just verify` succeeds from a clean checkout on supported Linux and macOS hosts.
- The approved regression archive replays to the committed state hash on two clean machines.
- No canonical event is silently discarded; unknown variants are quarantined and observable.
- Duplicate delivery, crash/restart, and replay yield identical state and feature hashes.
- Every live signal has a complete evidence bundle, valid data health, model/build provenance, capacity estimate, and explicit invalidation rules.
- Historical scores, clusters, features, and signals pass point-in-time leakage tests.
- The execution simulator accounts for observed book state, latency, partial fills, fees, funding, impact, and exit liquidity.
- Model promotion cannot bypass locked holdout, signature, schema, role, and policy checks.
- API responses preserve decimal values exactly and carry watermark, schema, and health metadata.
- The Swift client resumes streams without sequence regression, marks stale state, and never presents cached data as current.
- Backup restoration produces known archive, PostgreSQL, ClickHouse, and RocksDB checkpoint hashes.
- `hl-exec` is absent from V1 release manifests, packages, services, and deployed hosts.

## Recommended Execution Mode

Use subagent-driven development with one fresh implementation worker per task and two review gates:

1. **Requirement review:** verify the task satisfies its interfaces and acceptance checks.
2. **Code-quality review:** verify determinism, type safety, error handling, test quality, performance impact, and documentation.

Tasks within a plan are sequential unless the plan explicitly marks them parallel. Separate plans do not begin before the previous stage gate is recorded in `docs/stage-gates/`.

## Commit and Branch Discipline

- Use these exact stage branch names: `stage/0-foundations`, `stage/1-truth`, `stage/2-state`, `stage/3-intelligence`, `stage/4-market`, `stage/5-research`, `stage/6-desk`, and `stage/7-production-hardening`.
- One focused commit per task; migrations and generated schema outputs belong to the task that introduces them.
- Commit subjects use Conventional Commit form with an imperative description, for example `feat(capture): persist source observations before acknowledgement`, `test(state): add duplicate-delivery regression`, or `docs(gate): record Stage 2 approval evidence`.
- No force-push after a stage-gate review begins.
- Each stage ends with a signed annotated tag: `stage-0-foundations`, `stage-1-truth`, `stage-2-state`, `stage-3-intelligence`, `stage-4-market`, `stage-5-research`, and `stage-6-desk`. Configure the operator-controlled Git signing identity before Stage 0; unsigned stage tags are invalid.
- Production release candidates use signed tags: `v0.1.0-rc.1`, followed by `v0.1.0` after the final release gate.

## Stage-Gate Record Format

The exact stage records are `docs/stage-gates/stage-0-foundations.md`, `stage-1-truth-layer.md`, `stage-2-state-reconstruction.md`, `stage-3-wallet-entity.md`, `stage-4-market-intelligence.md`, `stage-5-alpha-laboratory.md`, and `stage-6-internal-desk.md`.

Each record contains these fields with concrete values generated during that gate:

- A title naming the stage and outcome.
- `Design specification commit`: the 40-character SHA returned by `git rev-list -n 1 design-approved-v1.0.0`.
- `Implementation commit`: the clean pre-gate commit returned by `git rev-parse HEAD` before verification starts.
- `Toolchain manifest hash`: the lowercase 64-character SHA-256 emitted by `stage-gate`.
- `Regression archive manifest hash`: the lowercase 64-character SHA-256 emitted by `stage-gate`, or the literal `not-applicable-stage-0` for Stage 0.
- `Verification command`: the exact `just stage-0-gate` through `just stage-6-gate` command.
- `Verification result`: the literal `PASS` only when every configured check succeeds.
- `Known limitations`: a bounded list; use the literal `none` only after reviewers confirm there are no known limitations.
- Platform/data approval and independent review: signer identity, UTC RFC 3339 timestamp, and detached-signature artifact path.
- Research, risk, security, or product approvals when the stage plan requires those roles.

Gate tooling and configuration are committed first. Verification then runs from that clean commit and writes machine evidence under ignored `target/stage-gates/`. The approval record and a copy of the canonical JSON evidence are committed afterward, and the signed stage tag points to that evidence commit. The first implementation task in each subsequent plan verifies both the signed tag and the preceding gate record.

## Completion Definition

The V1 program is complete only when:

- Stages 0 through 6 pass their documented gates.
- The production-hardening plan passes security, restore, load, soak, chaos, and canary checks.
- Analysts can inspect a live signal, reconstruct its evidence, enter a decision, and review point-in-time outcomes without hidden future data.
- The system can be rebuilt from immutable archives and signed configuration on a clean operator-controlled environment.
- The final release contains no execution capability or signing material.
