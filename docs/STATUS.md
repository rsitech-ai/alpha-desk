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
| 1 — Truth layer | Durable synthetic runtime mechanics are locally proven; stage not passed | Validated byte-preserving observations, strict capture configuration, crash-safe append-only spool, durability receipts, recovery scanner, immutable hash-chained close manifests, primary-node file adapters, exhaustive source-trust admission, deterministic canonical identity and public trade mapping, V1 upcast validation, bounded gap/duplicate/divergence sequencing, chain-scoped canonical/raw Parquet archive, atomic verified manifest chains, corruption-before-yield replay reads, idempotent retained-generation compaction, DataFusion inspection/count tooling, archive-before-journal-before-JetStream-before-cursor coordination, PostgreSQL recovery, owned runtime lifecycle, atomic status, deterministic fixture replay, one-restart process E2E, and bounded synthetic soak evidence | Real committed-node mapper and source loop, spool-to-runtime wiring, real-node/operator-corpus qualification, independent/recovery/operator/public/historical transports, remaining source mappings, real historical upcasts, crash-failpoint restart matrix, loopback health/metrics, multi-hour soak, production TLS/identity/replicated JetStream qualification, and signed gate |
| 2 — State reconstruction | Scaffold-only | Workspace crate boundaries | Deterministic reducers, checkpoints, correction handling, reconciliation, replay, and signed gate |
| 3 — Wallet/entity intelligence | Scaffold-only | Workspace crate boundaries | Wallet metrics, entity graph, attribution, confidence, and signed gate |
| 4 — Market intelligence/signals | Scaffold-only | Workspace crate boundaries | Feature families, signal lifecycle, health gating, evaluation, and signed gate |
| 5 — Alpha laboratory | Scaffold-only | Workspace crate boundaries | Experiment registry, walk-forward evaluation, leakage controls, models, promotion, and signed gate |
| 6 — Internal desk | Scaffold-only | Swift library package and service package boundaries | REST/WebSocket API, macOS/iOS apps, cache/resume behavior, evidence panels, security review, restore drill, and signed gate |
| 7 — Production hardening/release | Planned with partial Stage 0 scaffolding | CI workflows, reproducibility tooling, infrastructure skeleton, dependency/security policy | SLOs, backup/restore evidence, load/soak/chaos, security audit, canary/rollback, deterministic public export, legal decision, public history, and release approvals |

## Runnable surface

The currently useful runnable components are engineering tools and one explicit
synthetic capture runtime:

- `stage-gate`
- `schema-check` and `schema-generate`
- `fixture-inspect`
- `spool-inspect`
- `canonical-inspect`
- `archive-inspect`
- `architecture-check`
- `build-info`
- `hl-capture check-config`
- `hl-capture status --json`
- `hl-capture fixture-replay`

`hl-capture fixture-replay` constructs the real local Parquet archive,
PostgreSQL progress adapter, authenticated JetStream publisher, coordinator,
status writer, cancellation tree, and signal handler. The self-contained E2E
restarts that process once against the same durable state and the soak wrapper
retains bounded JSON evidence. This lane is explicitly synthetic and does not
exercise the configured primary-node adapter or source spool. The production
`hl-capture run` command fails closed with
`capture_runtime.committed_source_mapper_unavailable`; it does not silently
substitute fixture or public data.

The primary-node adapters remain focused-test proven against normalized
official examples, not qualified against operator node recordings.
`spool-inspect` can verify retained segments, manifests, and one complete open tail; it
is an operator tool, not a capture service. `hl-analytics` exports the local
immutable archive, verified compaction, and full-chain inspection libraries;
`archive-inspect` verifies reachable canonical/raw objects and independently
counts canonical Parquet rows through DataFusion. `just dev-up` starts
dependencies only. The Swift package exports foundation libraries and has no
application executable. Therefore the repository has truth-layer runtime
mechanics evidence, not a live-source product E2E or desk application.

The block-batched public trade fixture now maps deterministically through a
versioned market catalog, but remains auxiliary `ProvisionalSource` evidence.
It does not establish committed history or production node compatibility.

## Current release blockers

- `blocked:license-decision` — Apache-2.0 is current; any dual-license change requires owner/legal approval.
- `blocked:external` — trusted identities, signed approvals, a second builder, hosted CI evidence, tags, canonical organization repository creation, and publication.
- `blocked:public-history` — the current recovery/engineering history contains transport refs and author metadata that require a deliberate sanitized export decision.
- `blocked:live-qualification` — the committed node-directory/source-spool
  runtime, one clean restart, and bounded synthetic node-format soak are locally
  proven. Action-bearing source semantics, raw-observation Parquet archival,
  independent-source recovery, disk-reserve enforcement, multi-hour/load/host
  restart evidence, API, and UI are still absent.

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
- `cargo +1.97.1 test -p hl-capture --test committed_pipeline --locked --offline`
- `cargo +1.97.1 test -p hl-capture --test source_spool --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test event_id --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test input --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test block --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test node_mapping --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test upcast --locked --offline`
- `cargo +1.97.1 test -p hl-capture --test sequencer --locked --offline`
- `cargo +1.97.1 test -p hl-capture --locked --offline`
- `cargo +1.97.1 test -p hl-analytics --test archive --locked --offline`
- `cargo +1.97.1 test -p archive-inspect --locked --offline`
- `just postgres-migration-smoke`
- `just capture-e2e` — fresh PostgreSQL/NATS, three raw node-format
  observations, three committed empty blocks/publications, two verified closed
  spool segments across one clean restart, verified archive, clean shutdown
- `just capture-soak 10s` — ten drip-fed raw node-format observations, ten
  committed empty blocks/publications, one restart, verified spool/archive,
  clean shutdown
- `just spool-verify`
- `cargo +nightly-2026-07-16 fuzz run spool_segment fixtures/spool/valid-v1 -- -max_total_time=60`

The capture reports are retained under ignored
`target/evidence/capture-e2e/`; both declare
`"mode": "synthetic-node-source"` and `"live_source_qualified": false`.
The archive summaries currently report zero raw observations because the
long-term raw Parquet writer is not connected yet; the verified local spool is
the retained raw evidence in this lane. The Compose smoke verified NATS, ClickHouse, PostgreSQL, MinIO, the OpenTelemetry
Collector, and VictoriaMetrics, then removed its uniquely owned containers,
volumes, and network.

## Evidence discipline

- Local green checks are not hosted-CI proof.
- Dependency-stack uptime is not product-runtime proof.
- Service uptime before durable spool/recovery/continuity/archive exists is not truth-layer evidence.
- A dashboard backed only by provisional public data is not the approved venue-wide truth layer.
- No stage may be described as passed until its exact gate contract and signed evidence verify.
