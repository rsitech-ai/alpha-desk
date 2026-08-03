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
| 1 — Truth layer | Empty committed-block runtime and one-way failover are synthetic-source proven; stage not passed | Validated byte-preserving observations, strict primary/independent topology, crash-safe hash-chained per-source spools, exact-height create-once failover state, bounded one-record-at-a-time spool verification/replay and central canonical drain, primary and independent node-directory adapters, empty committed-block mapping, exhaustive source-trust admission, deterministic canonical identity, bounded sequencer, canonical/raw Parquet archive with bounded raw batches, raw-segment provenance and parity, archive-before-journal-before-JetStream-before-cursor coordination, reconnecting PostgreSQL/JetStream sessions, absolute and percentage disk gates, V3 active-source/failover/backlog/capacity status, bounded staggered reconnect backoff, owned runtime lifecycle, restart and no-failback E2E, PostgreSQL/NATS outage-recovery E2E, and bounded synthetic soak evidence | Qualified action-bearing committed mapping and operator corpus, separately operated independent-source qualification, recovery/operator/public/historical transports, overlap reconciliation and explicit failback procedure, real historical upcasts, crash-failpoint matrix, loopback health/metrics, multi-hour soak, production TLS/identity/replicated JetStream qualification, and signed gate |
| 2 — State reconstruction | Block-atomic, local checkpoint, serial replay, exact canonical trade facts, and exact canonical order lifecycle implemented; stage not passed | Pure synchronous reducer contract, default-deny kind/schema ownership, contiguous committed watermark, deterministic canonical state bytes and hash, duplicate idempotence, whole-block rollback, bounded mutations, immutable state deltas, exact state-image restore, content-derived canonical checkpoint manifests bound to archive/schema/reducer identity, descriptor-relative private manifest-last local checkpoint publication/load, immutable-manifest serial replay with preflight, deterministic receipts, exact trade facts with ordinal participant legs and stored quantity-symmetry reconciliation, exact default-deny order lifecycle with hash-linked transitions, bounded synthetic trade/order replay and checkpoint evidence, and focused adversarial tests | Qualified complete action-bearing account/position/balance/fee/funding/transfer reducers, RocksDB atomic batch/checkpoints, production replay service runtime, correction handling, external account/book reconciliation, deployed-source rebuild evidence, and signed gate |
| 3 — Wallet/entity intelligence | Scaffold-only | Workspace crate boundaries | Wallet metrics, entity graph, attribution, confidence, and signed gate |
| 4 — Market intelligence/signals | Scaffold-only | Workspace crate boundaries | Feature families, signal lifecycle, health gating, evaluation, and signed gate |
| 5 — Alpha laboratory | Scaffold-only | Workspace crate boundaries | Experiment registry, walk-forward evaluation, leakage controls, models, promotion, and signed gate |
| 6 — Internal desk | Scaffold-only | Swift library package and service package boundaries | REST/WebSocket API, macOS/iOS apps, cache/resume behavior, evidence panels, security review, restore drill, and signed gate |
| 7 — Production hardening/release | Planned with partial Stage 0 scaffolding | CI workflows, reproducibility tooling, infrastructure skeleton, dependency/security policy | SLOs, backup/restore evidence, load/soak/chaos, security audit, canary/rollback, deterministic public export, legal decision, public history, and release approvals |

## Runnable surface

The currently useful runnable components are engineering tools and a narrow
synthetic-source capture runtime:

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
- `hl-capture run`
- `hl-capture fixture-replay`
- `state-replay fixture-e2e`
- `state-replay trade-e2e`
- `state-replay order-e2e`
- `state-replay archive-e2e`

`hl-capture run` constructs the real node-directory adapter, raw-first source
spool, local raw/canonical Parquet archive, PostgreSQL progress adapter,
authenticated JetStream publisher, coordinator, status writer, cancellation
tree, and signal handler. The committed mapper accepts only structurally valid
empty action bundles and fails closed on action-bearing records. Committed
node configuration now requires the node's `actions-and-responses` replica
profile and rejects action-only profiles, while correctly retaining live
qualification as blocked until the configured process and an operator corpus
prove the actual schema. Spool scans
and replay allocate at most one record body at a time, raw archival emits
bounded batches, and capture configuration rejects segment targets above
512 MiB. Local acquisition and canonical drain are independent owned tasks:
PostgreSQL or NATS failure degrades readiness without stopping fsynced source
capture, and the drain reconnects from durable PostgreSQL progress.
The atomic V3 status distinguishes active-source fsynced capture backlog from
downstream publication plans, reports source class and bounded source health,
records the immutable failover height/reason without source paths or operator
identity, reports the oldest pending capture height and lowest spool/archive
free-space percentage, warns below 20% free, and rejects new writes below 10%
or the configured absolute reserve. Independent operation can be yellow-ready
but can never become green.

`hl-capture fixture-replay` retains a deterministic coordinator-only lane. The
self-contained production-entrypoint E2E uses `hl-capture run`, restarts it once
against the same spool, archive, journal, and stream, and requires exact
spool/raw/block/publication parity. The soak wrapper retains bounded JSON
evidence. Both lanes are explicitly synthetic and do not qualify deployed-node
source semantics.

The primary-node adapter remains focused-test and synthetic-runtime proven
against normalized official examples, not qualified against operator node
recordings. `spool-inspect` can verify retained segments, manifests, and one
complete open tail; it is an operator tool, not a capture service.
`hl-analytics` exports the local immutable archive, verified compaction, and
full-chain inspection libraries; `archive-inspect` verifies reachable
canonical/raw objects and independently counts canonical Parquet rows through
DataFusion. `just dev-up` starts dependencies only. The Swift package exports
foundation libraries and has no application executable. Therefore the
repository has a narrow truth-layer runtime, not a live-source product E2E or
desk application.

`canonical-ledger` and `replay-engine` now provide the first Stage 2 slices: a
storage-neutral, synchronous block-atomic reducer boundary with a frozen
reducer-set identity, deterministic sorted state image, domain-separated state
hash, committed-height continuity, duplicate idempotence, and default-deny
event/schema support; an invisible prepare/explicit-commit boundary; a
storage-neutral atomic state port and `hl-core` store-before-visibility
coordinator; a private descriptor-relative local checkpoint store; and bounded
serial replay over explicitly ordered immutable archive manifests.
Replay preflights chain, range, schema, count, and starting-state compatibility,
applies only at block boundaries, and emits deterministic completed/cancelled
receipts. Focused tests prove that storage failure or a mismatched durable
receipt cannot advance the visible ledger. An exact canonical-semantic trade
reducer now stores one immutable trade fact, two ordinal participant legs, and
a quantity-symmetry assessment; replay tests prove checkpoint equivalence and
whole-block rollback. It does not infer participant roles or account/order
effects and is not deployed-source qualification. No qualified action-bearing
production reducer, RocksDB adapter, production archive/replay service, or
external reconciliation result exists yet. A bounded `state-replay fixture-e2e`
process now generates explicit
synthetic evidence for repeated rebuild, local checkpoint resume, and
poison-block atomicity. `state-replay archive-e2e` runs the same repeat/resume
proof read-only against an operator-selected canonical archive range after
freezing the current catalog into verified immutable manifests; it remains
watermark-only with source qualification explicitly unassessed.
`state-replay trade-e2e` generates canonical trade events and proves repeated
exact-state rebuild, decoded trade/participant/reconciliation cardinality,
private checkpoint resume, malformed-trade reducer failure, and
unsupported-schema quarantine. Its report explicitly declares synthetic
unassessed source evidence, Stage 1/2 false, and account/order/position
qualification false.
`state-replay order-e2e` generates all seven exact order event kinds and proves
repeated exact-state rebuild, decoded fact/current/transition cardinality,
private checkpoint resume, checked overfill rollback, terminal lifecycle
counts, and unsupported-schema quarantine. Its report marks only the synthetic
order contract proven; Stage 1/2, deployed/live source, position, margin, and
execution qualification remain false.
`state-replay market-e2e` generates the complete twelve-kind V1 market family
in valid prerequisite order. Its independently repeated full range includes
the hash-only metadata transition and proves identical unresolved final-state
and full receipt hashes, strict decoding of both metadata intervals after each
path, and a private checkpoint resume whose suffix crosses that transition.
It also proves unresolved value suppression, late invalid-transition
whole-block rollback, unsupported-schema quarantine, and owner-only evidence
permissions. Its report marks only the synthetic market contract proven;
Stage 1/2, deployed/live source, authoritative metadata, external oracle
reconciliation, account, position, margin, book, signal, and execution
qualification remain false.
Stage 2 remains unqualified. The exact current contract and limitations are
recorded in
[`docs/contracts/deterministic-state-v1.md`](contracts/deterministic-state-v1.md).
Operator commands and evidence interpretation are documented in
[`docs/runbooks/state-replay-evidence.md`](runbooks/state-replay-evidence.md).

The block-batched public trade fixture now maps deterministically through a
versioned market catalog, but remains auxiliary `ProvisionalSource` evidence.
It does not establish committed history or production node compatibility.

## Current release blockers

- `blocked:license-decision` — Apache-2.0 is current; any dual-license change requires owner/legal approval.
- `blocked:external` — trusted identities, signed approvals, a second builder, hosted CI evidence, tags, canonical organization repository creation, and publication.
- `blocked:public-history` — the current recovery/engineering history contains transport refs and author metadata that require a deliberate sanitized export decision.
- `blocked:live-qualification` — the committed node-directory/source-spool
  runtime, one clean restart, and bounded synthetic node-format soak are locally
  proven, including raw Parquet parity and enforced absolute disk reserve.
  Action-bearing source semantics, separately operated independent-source
  qualification, retained-overlap reconciliation, multi-hour/load/host restart
  evidence, API, and UI are still absent.

No validated secret exposure was found by the 2026-07-28 local audit, but normal secret scanning is not sufficient to approve encoded archives or every remote ref for publication.

## Latest local evidence

- `just state-replay-account-e2e 30 12 4` — retained private synthetic account
  report proves repeat/resume equivalence, exact account-flow/relation/mode
  namespace cardinalities, typed prerequisite denial, and atomic late-invalid
  rollback. It is not deployed/live source, authoritative balance, position,
  episode, liquidation, settlement, funding-attribution, margin-model, or
  Stage 1/2 qualification.

The following checks passed most recently on 2026-07-29. They establish local code and
dependency-stack evidence only; they do not close the signed Stage 0 gate or
prove a running Alpha Desk product:

- `just verify`
- `just generated`
- `SOURCE_DATE_EPOCH=1784894400 just reproducible`
- `just stage-0-compose-smoke`
- `just state-replay-trade-e2e 12 5 4` — retained private report
  `target/evidence/state-replay-trade/20260729T180733Z-38718/report.json`
  proves exact synthetic canonical trade-state repeat/resume and atomic
  rejection boundaries; it does not qualify Stage 1, Stage 2, or live source.
- `just state-replay-order-e2e 20 8 4` — retained private report
  `target/evidence/state-replay-order/20260729T185537Z-82818/report.json`
  proves four identical rebuilds, checkpoint-equivalent resume, 80 immutable
  facts and transitions, 20 current orders, exact terminal counts, and atomic
  overfill/unsupported-schema rejection; it does not qualify Stage 1, Stage 2,
  deployed/live source, position, margin, or execution semantics.
- `just state-replay-market-e2e 20 8 4` — retained private report
  `target/evidence/state-replay-market/20260729T211159Z-40107/report.json`
  proves four identical full-range rebuilds through the hash-only metadata
  transition, a checkpoint suffix crossing that transition to the same
  unresolved final hash, 119 immutable facts, two strictly decoded metadata
  intervals, absent exact-value applicability, metadata-unresolved
  suppression, and atomic invalid/unsupported rejection. Every retained
  directory is `0700`, every file is `0600`, and only
  `synthetic_market_contract_proven` is true; Stage 1/2 and all deployed/live
  or downstream qualifications remain false. The prior pre-remediation report
  remains retained at
  `target/evidence/state-replay-market/20260729T202402Z-10401/report.json` but
  does not satisfy the combined repeat/resume-through-metadata boundary.
- `just oss-audit`
- `gitleaks detect --source . --no-banner --redact --exit-code 1`
- `cargo +1.97.1 test -p hl-capture --test spool_recovery --locked --offline`
- `cargo +1.97.1 test -p spool-inspect --locked --offline`
- `cargo +1.97.1 test -p hl-protocol --test node_golden --locked --offline`
- `cargo +1.97.1 test -p hl-protocol --test source_trust --locked --offline`
- `cargo +1.97.1 test -p hl-capture --test node_adapter --locked --offline`
- `cargo +1.97.1 test -p hl-capture --test committed_pipeline --locked --offline`
- `cargo +1.97.1 test -p hl-capture --test source_spool --locked --offline`
- `cargo +1.97.1 test -p hl-capture --test raw_segment_archive --locked --offline`
- `cargo +1.97.1 test -p hl-capture --test disk_reserve --locked --offline`
- `cargo +1.97.1 test -p canonical-events --test event_id --locked --offline`
- `cargo +1.97.1 test -p canonical-ledger --locked --offline`
- `cargo +1.97.1 clippy -p canonical-ledger --all-targets --all-features --locked --offline -- -D warnings`
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
  observations in both spool and Parquet, three committed empty
  blocks/publications, two verified closed spool segments across one clean
  restart, verified archive, clean shutdown
- `just capture-outage-e2e` — the production entrypoint remains alive while
  disposable NATS and PostgreSQL containers are paused in turn; the verified
  spool grows to three and then five records, V3 status reports two pending
  capture records with the exact oldest height during each outage, health
  becomes non-ready yellow, and restoration catches up to five raw records,
  blocks, and acknowledged publications exactly once with zero final backlog
- `just capture-failover-e2e` — two five-record source spools and ten raw
  observations prove an exact second-height primary gap, create-once failover,
  five contiguous blocks/publications, clean restart, repaired-primary capture,
  zero final active backlog, and no automatic failback
- `just capture-soak 30s` — thirty drip-fed raw node-format observations,
  thirty raw Parquet observations, thirty committed empty
  blocks/publications, one restart, zero final backlog, verified
  spool/archive, clean shutdown
- `just spool-verify`
- `cargo +nightly-2026-07-16 fuzz run spool_segment fixtures/spool/valid-v1 -- -max_total_time=60`

The latest V2 E2E reports are retained under ignored
`target/evidence/capture-e2e/`; restart and soak reports declare
`"mode": "synthetic-node-source"`, while the fault report declares
`"mode": "synthetic-node-source-dependency-outage"` and the failover report
declares `"mode": "synthetic-dual-source-failover"`. Every report retains
`"live_source_qualified": false`, the V3 status schema, final disk capacity,
and final active-source backlog.
The archive summaries require raw observation parity with the spool and
canonical block count. The Compose smoke verified NATS, ClickHouse, PostgreSQL, MinIO, the OpenTelemetry
Collector, and VictoriaMetrics, then removed its uniquely owned containers,
volumes, and network.

## Evidence discipline

- Local green checks are not hosted-CI proof.
- Dependency-stack uptime is not product-runtime proof.
- Service uptime before durable spool/recovery/continuity/archive exists is not truth-layer evidence.
- A dashboard backed only by provisional public data is not the approved venue-wide truth layer.
- No stage may be described as passed until its exact gate contract and signed evidence verify.
