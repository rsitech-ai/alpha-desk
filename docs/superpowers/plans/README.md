# Hyperliquid Alpha Desk Implementation Plan Set

The approved production design is implemented through eight gated plans. Execute them in order; no later stage starts before the preceding signed stage tag and approval record verify.

## Canonical Sources

- Approved design: `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md`
- Approved design tag: `spec-v1.0.0`
- Program roadmap: `docs/superpowers/plans/2026-07-24-00-hyperliquid-alpha-desk-program-roadmap.md`
- Specification traceability: `docs/superpowers/plans/2026-07-24-99-spec-traceability.md`

## Execution Order

| Order | Stage | Plan | Signed completion tag |
|---:|---|---|---|
| 0 | Foundations | `2026-07-24-01-foundations.md` | `stage-0-foundations` |
| 1 | Truth layer | `2026-07-24-02-truth-layer.md` | `stage-1-truth` |
| 2 | State reconstruction | `2026-07-24-03-state-reconstruction.md` | `stage-2-state` |
| 3 | Wallet and entity intelligence | `2026-07-24-04-wallet-entity-intelligence.md` | `stage-3-intelligence` |
| 4 | Market intelligence and signals | `2026-07-24-05-market-intelligence-signals.md` | `stage-4-market` |
| 5 | Alpha laboratory | `2026-07-24-06-alpha-laboratory.md` | `stage-5-research` |
| 6 | Internal desk | `2026-07-24-07-internal-desk.md` | `stage-6-desk` |
| 7 | Production hardening and release | `2026-07-24-08-production-hardening.md` | `v0.1.0-rc.1`, then `v0.1.0` |

The future execution enclave is intentionally absent. It requires a separate threat model, design approval, and implementation plan after shadow-live evidence satisfies the admission criteria in design section 22.

## Shared Type Ownership

| Contract | Owning crate/module | Consumers |
|---|---|---|
| Checked fixed-point values, probability, IDs, protocol/known time, intervals, horizons, directions, latency, calibration, block ranges | `domain-types` | Every Rust service and generated API binding |
| Canonical event envelope, event families, confirmation, stable event identity | `canonical-events` | Capture, state, archive, replay, feature computation |
| Data-health state and scoped health assessment | `telemetry::health` | State, features, signals, API, client contracts |
| Bitemporal feature keys/values/snapshots and content-addressed evidence references | `feature-core` | Wallet/entity, market, signal, research, API |
| Wallet/entity subject, applicability, statistical skill, copyability, capacity, portfolio-context summary | `wallet-intelligence` | Market intelligence, research, API |
| Temporal graph nodes, evidence, cluster versions, independence weights | `entity-graph` | Wallet intelligence, market intelligence, research |
| Sentiment vector, cohort/ratio scope, regime, crowding, fragility, market memory | `market-intelligence` | Signal engine, research, API |
| Signal object, evidence bundle, lifecycle, invalidation, utility, outcome | `signal-core` | Research, API, Swift client |
| Experiment manifest, replay receipt, promotion report, model bundle/registry | `hl-research` and `model-runtime` | Governance, API, production operations |
| OpenAPI/Protobuf DTOs and stream-resume semantics | `api-contracts` | `hl-api`, generated Swift client, integration tests |
| Swift display models, cache envelopes, data-health presentation | `DeskDomain` | Native desk packages only; never canonical computation |

A type is not redefined downstream. Cross-stage changes require the compatibility decision described in the roadmap freeze points.

## Execution Discipline

1. Create an isolated worktree from the approved baseline using `superpowers:using-git-worktrees`.
2. Use `superpowers:subagent-driven-development` for one fresh implementation worker per task, or `superpowers:executing-plans` for inline batches.
3. Follow every checkbox in a task in order: failing test, failure verification, minimal implementation, passing verification, focused commit.
4. Run the stage gate from a clean commit. Gate output is written under ignored `target/stage-gates/`.
5. Commit generated evidence and the signed approval record, then create the signed stage tag.
6. Do not weaken a gate to make a failing implementation pass. Change requirements only through an explicit design/spec revision.

## V1 Safety Boundary

The V1 repository, packages, manifests, services, hosts, API routes, Swift code, and tests contain no `hl-exec`, private keys, signer integration, order placement, automatic copy trading, or custodial function. `POST /v1/execution-estimates` performs simulation only.
