# Hyperliquid Alpha Desk Specification Traceability

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement the referenced plan task-by-task. This document is a coverage index, not a substitute for a task plan.

**Goal:** Demonstrate that every approved design section has an implementation owner, verification path, and explicit scope disposition.

**Architecture:** Coverage follows the same staged dependency chain as the implementation roadmap. Requirements that cut across stages are enforced both in the introducing task and in stage/release gates.

**Tech Stack:** The stack and exact version lines are owned by the approved design and `2026-07-24-01-foundations.md`.

## Global Constraints

- The canonical requirement source is `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md` at `spec-v1.0.0`.
- “Covered” means a named task creates the behavior and a named stage or release gate verifies it.
- The future execution boundary is deliberately deferred, not omitted accidentally.
- A specification change requires a new reviewed design revision and an updated traceability row before implementation.

---

## Coverage Matrix

| Design section | Primary implementation owner | Verification and disposition |
|---|---|---|
| 1. Executive decision | Program roadmap; all stage plans | V1 remains a private read-only alpha desk; release inventory proves no execution capability. |
| 2. Scope | Program roadmap; Foundations Task 1; Production Hardening Tasks 9–10 | Stage and final gates scan packages, routes, manifests, and hosts for excluded capabilities. |
| 3. Success criteria | Roadmap quality gates; each stage acceptance gate; Production Hardening Task 10 | Final release evidence aggregates product, data, research, and operational results without claiming guaranteed profitability. |
| 4. Architectural principles | Foundations Tasks 2–6; Truth Tasks 3–8; State Tasks 1–9; Alpha Lab Tasks 1–7 | Determinism, archive-first delivery, exact effects, fixed point, point-in-time evaluation, shared live/replay paths, fail-closed behavior, and evidence views are direct test targets. |
| 5. Hyperliquid-specific constraints | Truth Tasks 1–7; State Tasks 2–7; Production Hardening Task 3 | Venue-wide node/L1 capture, mode-aware accounts, dynamic markets, margin adapters, source priority, and audit-preserving latency are verified with golden and reconciliation fixtures. |
| 6. Technology baseline | Foundations Tasks 1, 6–9; Production Hardening Tasks 1–3 | Toolchains and dependencies are pinned, architecture checked, supply-chain scanned, and deployed without Kubernetes on the hot path. |
| 7. System context and trust boundaries | Foundations Tasks 6–9; Internal Desk Tasks 1–4; Production Hardening Tasks 1, 3, 5, 7 | Network/host/service boundaries, authentication, authorization, provenance, and source trust hierarchy are exercised in security and chaos evidence. |
| 8. Production deployment topology | Production Hardening Tasks 1–4, 8–10 | Ansible/systemd/Podman topology, Tokyo primary placement, independent secondary source, capacity, canary, and rollback are release-gated. |
| 9. Repository and code architecture | Foundations Tasks 1, 2, 6, 9 | Monorepo layout, dependency direction, deterministic interfaces, generated contracts, and service boundaries are machine checked. |
| 10. Acquisition and canonical event pipeline | Truth Tasks 1–10 | Source adapters, spool, sequencing, stable IDs, confirmation, gaps/divergence, schema evolution, subjects, archive-before-publish, recovery, and Stage 1 acceptance are all explicit tasks. |
| 11. Canonical domain model | Foundations Tasks 2–4; Truth Tasks 3–5 | IDs, event families/envelope, bitemporal fields, fixed-point values, schema compatibility, and golden events are contract-tested. |
| 12. Deterministic state reconstruction | State Tasks 1–10 | Atomic reducers, partitioning, positions, account modes, reconciliation, checkpoints, crash recovery, and Stage 2 state hashes are verified. |
| 13. Order book and executable liquidity | State Task 7; Alpha Lab Tasks 3–4 | L4 invariants, snapshot recovery, execution-price queries, partial fills, impact, and book-health suppression are tested in live and replay paths. |
| 14. Storage design | Truth Tasks 6–8; State Tasks 8–9; Wallet Task 10; Production Hardening Tasks 2, 4, 6 | Parquet, RocksDB, ClickHouse, PostgreSQL, retention, backup, restore, rebuild, and capacity evidence are release-gated. |
| 15. Wallet/account/entity intelligence | Wallet and Entity Intelligence Tasks 1–10 | Performance, whale components, Bayesian skill, style, intent, copyability, hard/soft links, independence, leader/follower, counterparty, and change detection have point-in-time tests and audit fixtures. |
| 16. Market intelligence and sentiment | Market Intelligence Tasks 1–6, 10 | Scoped ratios, Smart Flow, informed aggression, divergence, conviction, regime, crowding, pain, fragility, memory, and cross-asset context are formalized and stage-gated. |
| 17. Signal framework | Market Intelligence Tasks 7–10 | Immutable signals, lifecycle, evidence, utility, deduplication, suppression, invalidation, outcomes, and the three V1 families are regression-tested; other families remain research-only. |
| 18. Research/backtesting/profitability validation | Alpha Laboratory Tasks 1–7, 10 | Manifest registration, point-in-time store, labels, walk-forward/holdout, metrics, promotion defaults, execution simulator, baselines, and decay checks are mandatory. |
| 19. Model architecture and governance | Alpha Laboratory Tasks 7–10 | Approved model classes, signed bundles, registry states, isolated local inference, structured explanations, and device-personalization limits are verified. |
| 20. API specification | Internal Desk Tasks 1–5, 12 | REST/OpenAPI, gRPC, streams, exact decimals, watermarks, health, auth/RBAC, budgets, resume semantics, and contract examples are generated and tested. |
| 21. Native SwiftUI desk | Internal Desk Tasks 6–12 | macOS-first architecture, command center, market/wallet/entity/tape/replay views, local notifications, shadow portfolios, decision journal, accessibility, and iOS companion constraints are covered. |
| 22. Future execution boundary | Program roadmap; README V1 safety boundary; Production Hardening Tasks 7, 9–10 | Deferred by approval. V1 scans prove absence of signer/order routes. A new threat model, design, and plan are required before any execution work. |
| 23. Security specification | Foundations Task 6; Internal Desk Tasks 1–4; Production Hardening Tasks 1, 3, 5, 7, 9–10 | Threat model, network/host/application/supply-chain/data controls, tamper-evident audit, privacy, and independent security gate are explicit release evidence. |
| 24. Observability, health, and SLOs | Foundations Task 5; every stage gate; Production Hardening Tasks 2, 4, 8–10 | Health states/thresholds, metrics, SLOs, capacity, degraded behavior, and alert severities are exercised under load, soak, and failure. |
| 25. Testing and verification | Every task; all stage gates; Production Hardening Tasks 5–10 | Unit, property, golden, differential, fuzz, concurrency, load, soak, chaos, model, Swift, and release verification are named commands with expected results. |
| 26. Operational runbooks | Truth Task 9; State Task 9; Wallet Task 10; Market Task 10; Alpha Lab Tasks 9–10; Internal Desk Task 12; Production Hardening Tasks 1–8 | Required incident, recovery, data, model, API, client, backup, and canary procedures are created and exercised. |
| 27. Delivery stages and gates | Program roadmap; final task of each stage plan; Production Hardening Task 10 | Ownership, evidence format, clean-commit verification, approvals, signed tags, and separately approved Stage 7 are enforced. |
| 28. Open-source release strategy | Production Hardening Task 10 | Public/private split, Apache-2.0 licensing, secret/provenance review, reproducible build, contribution/security docs, plugin boundaries, and alpha-leak audit are release requirements. |
| 29. Key architecture decisions | Program roadmap freeze points; Foundations Tasks 1, 6, 9; plan-specific ADRs | Decisions are encoded in dependency checks, contracts, deployment manifests, and ADRs rather than left as narrative only. |
| 30. Risks and mitigations | Wallet Tasks 3–9; Market Tasks 1–8; Alpha Lab Tasks 2–9; Production Hardening Tasks 2, 4–10 | Intent uncertainty, skill decay, cluster inflation, execution costs, margin-model drift, overfitting, complexity, mobile limits, storage growth, and alpha leakage each have a test or operating control. |
| 31. Final product definition | Program roadmap; Internal Desk Tasks 4–11; Production Hardening Tasks 8–10 | Release acceptance demonstrates a live, evidence-complete, point-in-time, portfolio-aware analyst workflow with local infrastructure and no execution. |
| 32. Specification review checklist | This traceability document; stage gates; Production Hardening Task 10 | Final gate rechecks scope, contracts, security, reproducibility, documentation, and unresolved limitations against the approved checklist. |

## Coverage Rules for Changes

1. A task-changing pull request identifies the affected design sections and traceability rows.
2. A new canonical type names exactly one owning crate and may not be redefined in generated or client code.
3. A new signal family enters research-only state first and cannot become live merely through configuration.
4. A relaxed promotion, health, security, or release threshold requires a versioned policy change, explicit rationale, independent approval, and a fresh locked holdout or release gate as applicable.
5. Any execution-related code stops V1 release and starts a separate design-review workflow.
