# Implementation Status

Updated: 2026-07-29

This is the evidence ledger for the current working repository. The approved design and stage plans describe the target system; unchecked plan items are not proof that work is absent, and checked items alone would not be proof that a gate passed. A stage is complete only when its required tests, evidence record, approvals, and signed tag verify.

## Readiness labels

- `implemented`: source and focused tests exist.
- `runtime-proven`: the relevant process or product path was exercised in its real runtime.
- `planned`: specified but not implemented.
- `blocked:external`: requires authority or evidence outside this local repository.
- `HOLD`: the stage gate has not produced a valid PASS outcome.

## Stage summary

| Stage | Current status | What exists | What is still required |
| --- | --- | --- | --- |
| 0 — Foundations | Local implementation checks and Compose smoke pass; gate `HOLD` | Workspace/toolchains, exact domain types, identifiers, Protobuf contracts, deterministic fixtures, telemetry/provenance, architecture checks, supply-chain policy, dependency stack, deployment scaffolding, gate tooling, concurrent child-output draining, and owned-resource cleanup proof | Replace placeholder trust identities; obtain second-builder, CI, reviewer, approval, clean evidence-commit, and signed-tag evidence |
| 1 — Truth layer | Tasks 1–7 have substantial library/tool foundations; stage not passed | Validated byte-preserving observations, strict capture configuration, crash-safe append-only spool, durability receipts, recovery scanner, immutable hash-chained close manifests, primary-node file adapters, exhaustive source-trust admission, deterministic canonical identity and public trade mapping, V1 upcast validation, bounded gap/duplicate/divergence sequencing, chain-scoped canonical/raw Parquet archive, atomic verified manifest chains, corruption-before-yield replay reads, idempotent retained-generation compaction, DataFusion inspection/count tooling, deterministic fixtures, and parser fuzz target | Real-node/operator-corpus qualification, independent/recovery/operator/public/historical transports, remaining source mappings, real historical upcasts, archive-before-PostgreSQL-cursor coordination, JetStream publication, long-running capture service, runtime/restart/soak evidence, and signed gate |
| 2 — State reconstruction | Scaffold-only | Workspace crate boundaries | Deterministic reducers, checkpoints, correction handling, reconciliation, replay, and signed gate |
| 3 — Wallet/entity intelligence | Scaffold-only | Workspace crate boundaries | Wallet metrics, entity graph, attribution, confidence, and signed gate |
| 4 — Market intelligence/signals | Scaffold-only | Workspace crate boundaries | Feature families, signal lifecycle, health gating, evaluation, and signed gate |
| 5 — Alpha laboratory | Scaffold-only | Workspace crate boundaries | Experiment registry, walk-forward evaluation, leakage controls, models, promotion, and signed gate |
| 6 — Internal desk | Scaffold-only | Swift library package and service package boundaries | REST/WebSocket API, macOS/iOS apps, cache/resume behavior, evidence panels, security review, restore drill, and signed gate |
| 7 — Production hardening/release | Planned with partial Stage 0 scaffolding | CI workflows, reproducibility tooling, infrastructure skeleton, dependency/security policy | SLOs, backup/restore evidence, load/soak/chaos, security audit, canary/rollback, deterministic public export, legal decision, public history, and release approvals |

## Runnable surface

The currently useful runnable components are engineering tools:

- `stage-gate`
- `schema-check` and `schema-generate`
- `fixture-inspect`
- `spool-inspect`
- `canonical-inspect`
- `archive-inspect`
- `architecture-check`
- `build-info`

The five service packages compile but their binaries exit immediately. `hl-capture` now exports
Stage 1 observation/configuration contracts, the durable spool, primary-node file adapters,
source-trust admission, and a bounded deterministic canonical sequencer, but its binary does not
construct an adapter, open a source, or
remain running. The adapters are
focused-test proven against normalized official examples, not qualified against operator node
recordings. `spool-inspect` can verify retained segments, manifests, and one complete open tail; it
is an operator tool, not a capture service. `hl-analytics` exports the local
immutable archive, verified compaction, and full-chain inspection libraries;
`archive-inspect` verifies reachable canonical/raw objects and independently
counts canonical Parquet rows through DataFusion. Neither is wired into a
long-running service or durable capture cursor. `just dev-up` starts dependencies only. The Swift
package exports foundation libraries and has no application executable. Therefore there is not
yet a product E2E or long-running soak path to claim.

The block-batched public trade fixture now maps deterministically through a
versioned market catalog, but remains auxiliary `ProvisionalSource` evidence.
It does not establish committed history or production node compatibility.

## Current release blockers

- `blocked:license-decision` — Apache-2.0 is current; any dual-license change requires owner/legal approval.
- `blocked:external` — trusted identities, signed approvals, a second builder, hosted CI evidence, tags, canonical organization repository creation, and publication.
- `blocked:public-history` — the current recovery/engineering history contains transport refs and author metadata that require a deliberate sanitized export decision.
- `blocked:runtime-evidence` — archive storage behavior has focused local evidence, but capture coordination, replay service, restart, API, UI, load, soak, restore, canary, and rollback evidence do not exist yet.

No validated secret exposure was found by the 2026-07-28 local audit, but normal secret scanning is not sufficient to approve encoded archives or every remote ref for publication.

## Latest local evidence

The following checks passed most recently on 2026-07-29. They establish local code and
dependency-stack evidence only; they do not close the signed Stage 0 gate or
prove a running Alpha Desk product:

- `just verify`
- `just generated`
- `SOURCE_DATE_EPOCH=1784894400 just reproducible`
- `just stage-0-compose-smoke`
- `just oss-audit`
- `gitleaks detect --source . --no-banner --redact --exit-code 1`
- `cargo +1.97.1 test -p hl-capture --test spool_recovery --locked --offline`
- `cargo +1.97.1 test -p spool-inspect --locked --offline`
- `cargo +1.97.1 test -p hl-protocol --test node_golden --locked --offline`
- `cargo +1.97.1 test -p hl-protocol --test source_trust --locked --offline`
- `cargo +1.97.1 test -p hl-capture --test node_adapter --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test event_id --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test input --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test block --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test node_mapping --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test upcast --locked --offline`
- `cargo +1.97.1 test -p hl-capture --test sequencer --locked --offline`
- `cargo +1.97.1 test -p hl-analytics --test archive --locked --offline`
- `cargo +1.97.1 test -p archive-inspect --locked --offline`
- `just postgres-migration-smoke`
- `just spool-verify`
- `cargo +nightly-2026-07-16 fuzz run spool_segment fixtures/spool/valid-v1 -- -max_total_time=60`

The Compose smoke verified NATS, ClickHouse, PostgreSQL, MinIO, the OpenTelemetry
Collector, and VictoriaMetrics, then removed its uniquely owned containers,
volumes, and network.

## Evidence discipline

- Local green checks are not hosted-CI proof.
- Dependency-stack uptime is not product-runtime proof.
- Service uptime before durable spool/recovery/continuity/archive exists is not truth-layer evidence.
- A dashboard backed only by provisional public data is not the approved venue-wide truth layer.
- No stage may be described as passed until its exact gate contract and signed evidence verify.
