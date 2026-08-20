# Hyperliquid full-coverage traceability

Maps every `HLCOV-*` requirement in the expansion spec to a planned task, a target test or check, and the evidence that would accept it.

Spec: [`../specs/2026-08-19-hyperliquid-full-coverage-expansion.md`](../specs/2026-08-19-hyperliquid-full-coverage-expansion.md)
Plan: [`2026-08-19-hyperliquid-full-coverage-plan.md`](2026-08-19-hyperliquid-full-coverage-plan.md)

Approved base design path `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md` is absent on this branch. Tags `design-approved-v1.0.0` and `spec-v1.0.0` still apply.

Read-only scope is unchanged: no `/exchange`, signing, private keys, order placement, or copy-trading execution.

## Binding notes for later tasks

- Manifest is `capabilities.toml` plus JSON schema (T02). Spec §9.1 already uses toml. Appendix B still says `capabilities.yaml`; do not treat yaml as the T02 target.
- Expansion SQL is `schemas/postgres/0100_*.sql` and `schemas/clickhouse/0100_*.sql` so it never collides with V1 `0001`-`0009`. Spec §19 still lists `0005`/`0009` names. Filenames follow `0100+`.
- T04 is blocked: do not copy hlscreen fixtures into this repo until that block lifts.
- T36 stays in `rsitech-ai/hlscreen`.
- Canonical events stay V1.1.0 additive. No V2.
- First release profile enables no third-party providers.
- Do not add duplicate `canonical-archive` or `canonical-state-store` crates.
- Do not create a second web app.

| ID | Spec | Task | Target test/check | Acceptance evidence |
| --- | --- | --- | --- | --- |
| HLCOV-SRC-001 | §1, §6.1, §30.3 | T13, T14, T21 | `just capture-e2e`; node trade/order fixtures | Venue-wide activity comes from committed node datasets, not a wallet poller. |
| HLCOV-SRC-002 | §6.1 | T03, T13 | source-catalog fixture; failover e2e | Independent committed source is configured and divergence is recorded. |
| HLCOV-SRC-003 | §6.1, §30.4 | T09, T12 | `just public-ws-replay`; info capture tests | REST/WS outputs are provisional/reconciled. They do not overwrite committed state. |
| HLCOV-SRC-004 | §6.1, §13.3 | T16 | S3 backfill fixture; gap records | Dataset limitations are stored and returned with coverage. |
| HLCOV-SRC-005 | §6.1, §30.5 | T03, T35 | provider-policy fixture; coverage check | Provider fields carry provenance. Community rows stay `DiscoveryOnly`. First release leaves T35 disabled. |
| HLCOV-SRC-006 | §6.2, §6.3 | T03, T17 | `cargo test -p canonical-events`; source-catalog tests | Five envelope classes exist. Observations are archived first. |
| HLCOV-SRC-007 | §3.1 | T02 | `just hyperliquid-coverage-check` | Manifest lists every in-scope source family, including optional/disabled ones. |
| HLCOV-SRC-008 | §10.3 | T05 | `cargo test -p hl-protocol`; unknown-variant fixture | Bytes preserved. No `f64` in canonical parse. State-affecting unknowns quarantine. |
| HLCOV-SRC-009 | §10.4 | T05, T23 | pagination collision fixture | Overlap, same-ms, and truncation cases pass. |
| HLCOV-SRC-010 | §12 | T08, T22 | scheduler tests; 429 emulator | P0 is never starved. Safety envelope is enforced. |
| HLCOV-SRC-011 | §12.4, §3.2 | T08, T39 | egress-policy fixture; read-only scan | Every egress ID has a bucket. Anonymous proxy rotation is absent. |
| HLCOV-SRC-012 | §13.1 | T13, T14 | node schema fixtures | Required node datasets parse and archive. |
| HLCOV-SRC-013 | §13.3 | T16 | S3 object-manifest fixture | Resume, ETag/hash, and gap status are recorded. |
| HLCOV-SRC-014 | §13.4 | T38 | `just full-coverage-soak`; disk-pressure chaos | ~100 GB/day design load is tested. Raw is retained after ClickHouse materialization. |
| HLCOV-PROTO-001 | §4.1, §30.13 | T02 | `just hyperliquid-coverage-check` | Enabled capabilities missing owner/fixture/parser/health fail the check. |
| HLCOV-PROTO-002 | §4.2 | T02 | capability-status fixture | Unknown status strings are rejected. |
| HLCOV-PROTO-003 | §4.3 | T33, T34 | OpenAPI fixture; dashboard coverage fields | Completeness dimensions appear on analytical responses. |
| HLCOV-PROTO-004 | §9.1 | T02 | schema validate; `render-docs --check` | `capabilities.toml` and JSON schema exist. Generated matrix matches. |
| HLCOV-PROTO-005 | §9.2, App B | T02 | missing-field fixture | Incomplete records fail validate. |
| HLCOV-PROTO-006 | §9.3 | T02 | `just hyperliquid-coverage-check` | Check is offline. Codegen does not require the network. |
| HLCOV-PROTO-007 | §14.1 | T17 | `cargo test -p canonical-events --test upcast` | Envelope remains V1. Additive 1.1.0. Tag reuse is rejected. |
| HLCOV-PROTO-008 | §14.2, §14.3, §30.11 | T17 | canonical-events fixtures | New kinds have committed evidence. Snapshots do not emit ledger events. |
| HLCOV-PROTO-009 | §14.4 | T17 | event-id fixture; correction replay | Corrections point at superseded IDs. Archive bytes are unchanged. |
| HLCOV-PROTO-010 | §8, §30.2, §28.17 | T04, T05, T31, T36 | differential fixture harness (T04 blocked) | T04 does not import hlscreen fixtures while blocked. Later parity/difference records are required. T36 is out of this repo. |
| HLCOV-PROTO-011 | §5, §28.10 | T02, T07, T17 | `just hyperliquid-coverage-check` | Listed domains are implemented or explicitly gated. |
| HLCOV-PROTO-012 | §10.2, §11 | T05, T06, T07, T10, T11 | `just public-api-fixtures`; `just public-ws-replay` | Documented `/info` and WS families are registered. Official WS limits are enforced. |
| HLCOV-CORE-001 | §7.1, §30.8 | T19 | architecture-check; workspace layout | Still five deployables. No extra service crates for these domains. |
| HLCOV-CORE-002 | §7.2 | T19, T27 | storage-port tests | Roles unchanged. Redis is absent unless later profiling adds a disposable cache. |
| HLCOV-CORE-003 | §3.2, §30.6 | T19, T37 | NATS subject fixture; no Kafka crate scan | JetStream remains the bus. |
| HLCOV-CORE-004 | §2.1, §30.9 | T09, T19 | archive-before-ack capture tests | Publication cannot precede durable archive. Replay uses the live paths. |
| HLCOV-CORE-005 | §13.2, §28.9 | T15 | `just l4-replay-e2e` | L4 hashes match. Derived L2 reconciles or quarantines. |
| HLCOV-CORE-006 | §16, §18.2 | T18 | `cargo test -p canonical-ledger`; state-replay e2e | Invariants hold on fixtures. Replay hashes match. |
| HLCOV-CORE-007 | §18 | T18 | reconciliation-finding fixture | Unknown state-affecting variants fail the affected health gate. |
| HLCOV-CORE-008 | §19.3 | T19 | state-schema replay migration tests | CF changes bump schema version and replay. |
| HLCOV-CORE-009 | §19.1, §19.2 | T09, T17, T24, T27 | parquet schema fixtures; archive-inspect | New families exist. Partitioning is not by account address. |
| HLCOV-CORE-010 | §19.4 | T03, T20, T26, T32 | `just postgres-migration-smoke` on `0100_*.sql` | Control tables exist under `0100+`. V1 `0001`-`0009` are untouched. |
| HLCOV-CORE-011 | §7, §27.2 | T19, T37 | hl-core runtime tests; outage e2e | Node/API/NATS/CH/PG failure behavior matches §27.2. |
| HLCOV-WALLET-001 | §15.1 | T20 | postgres `0100` wallet-registry migration tests | Registry tables and required fields exist. |
| HLCOV-WALLET-002 | §15.2, §28.5 | T21 | discovery fixture from node trades | Wallets appear from committed trades/orders without a leaderboard seed. |
| HLCOV-WALLET-003 | §15.3 | T22 | tier-policy fixture | Tier assignment is versioned and explainable. |
| HLCOV-WALLET-004 | §15.4, §28.8 | T23 | `just wallet-backfill-e2e` | Coverage and truncation are visible per dataset. Jobs resume. |
| HLCOV-WALLET-005 | §16 | T18, T30 | episode/PnL fixtures; analytics projection e2e | Cashflow-adjusted outputs. No raw account-value performance claim. |
| HLCOV-WALLET-006 | §28.6 | T22, T38 | scheduler capacity test; soak | 10,000 registry rows schedule inside configured budgets. |
| HLCOV-WALLET-007 | §28.7 | T23 | `just wallet-reconciliation-e2e` | Current positions/orders/balances reconcile within policy. |
| HLCOV-WALLET-008 | §15.1, §19.4 | T32 | watchlist schema tests | Existing watchlist schema is extended, not copied. |
| HLCOV-EVM-001 | §17.1 | T25 | local/S3 EVM capture fixtures | Local raw is primary. S3 fallback is explicit. RPC is not the archive. |
| HLCOV-EVM-002 | §17.2 | T24 | `cargo test -p hl-protocol`; architecture-check | Types live in `hl-protocol` unless a later measured Ethereum crate split is approved. No Ethereum crate in T01. |
| HLCOV-EVM-003 | §17.3 | T26 | EVM decode fixtures | System txs, token events, and CoreWriter actions index. |
| HLCOV-EVM-004 | §17.4 | T26 | ABI registry postgres tests | Unknown logs remain queryable. |
| HLCOV-EVM-005 | §17.5, §28.12 | T26 | `just cross-layer-reconciliation-e2e` | Core/EVM transfers and CoreWriter links exist. |
| HLCOV-EVM-006 | §30.12, §28.11 | T24, T25 | `just hyperevm-replay-e2e` | Blocks/txs/receipts/logs/system txs are archived and queryable. |
| HLCOV-ANALYTICS-001 | §19.5, §28.13 | T27 | ClickHouse `0100` migration smoke; projection rebuild fixture | Fact tables exist. Archive remains the source of rebuild. |
| HLCOV-ANALYTICS-002 | §7, §28.13 | T28 | `just analytics-projection-e2e` | Rebuild from archive matches committed projections. |
| HLCOV-ANALYTICS-003 | §7, §28.14 | T29 | hl-analytics runtime tests | Existing intelligence crates are fed from production facts. |
| HLCOV-ANALYTICS-004 | §20.1 | T30 | wallet-metric projection fixtures | Listed wallet projections are produced with evidence IDs. |
| HLCOV-ANALYTICS-005 | §20.2, §28.15 | T30 | ranking fixture; PIT leakage tests | Rankings are cashflow-adjusted and coverage-labelled. |
| HLCOV-ANALYTICS-006 | §20.3 | T31 | market aggregate fixture | `coverage_*` fields are present. Tracked stats are not labelled global without venue-wide reconstruction. |
| HLCOV-ANALYTICS-007 | §20.4 | T31, T35 | provider-observation fixture | Provider PnL/labels stay separate from canonical reconstruction. |
| HLCOV-ANALYTICS-008 | §21, §28.16 | T32 | `just alert-lifecycle-e2e` | Lifecycle and evidence bundles exist. LLM output cannot confirm an alert. |
| HLCOV-API-001 | §22.1 | T33 | OpenAPI fixture | Envelope fields are required on analytical responses. |
| HLCOV-API-002 | §22.2-§22.5 | T33 | `just public-api-fixtures` | Listed GET routes exist or are explicitly gated. |
| HLCOV-API-003 | §22.6 | T33, T32 | OpenAPI fixture; watchlist POST tests | Control writes do not hit `/exchange`. |
| HLCOV-API-004 | §22.7 | T33 | stream resume fixture | Reconnect cannot present stale data as current. |
| HLCOV-API-005 | §23 | T34 | existing AlphaDesk / operator dashboard tests | Four workspaces are present in the existing apps. |
| HLCOV-API-006 | §28.20 | T34 | dashboard health/coverage fixture | Health, coverage, and limitations are visible. |
| HLCOV-API-007 | §23 | T34 | workspace layout check | No second web application crate/app is added. |
| HLCOV-API-008 | §3.2 | T33 | OpenAPI fixture review | GraphQL is not introduced as the primary contract. |
| HLCOV-OPS-001 | §3.2, §30.14 | T01, T39 | `just hyperliquid-full-coverage-docs`; read-only release scan | Spec/plan/docs check preserve read-only language. Release graph has no signer/`/exchange`. |
| HLCOV-OPS-002 | §3.2 | T16, T23, T33 | coverage fields in API fixture | Truncated sources cannot be reported as complete. |
| HLCOV-OPS-003 | §3.2, §30.7 | T39 | workspace/deployable scan | Production deployables remain Rust. |
| HLCOV-OPS-004 | §24 | T35, T39 | license/redaction fixture; secret scan | Provider secrets stay in adapter/egress. Labels do not overwrite protocol roles. |
| HLCOV-OPS-005 | §24, §28.18 | T39 | read-only release scan | Release artifacts have no execution/signing/secrets. |
| HLCOV-OPS-006 | §25 | T37, T38 | source-health metrics fixture; SLO evidence | Metrics in §25.4 exist. SLO numbers are measured. |
| HLCOV-OPS-007 | §26 | T02-T40 | `just` recipes in §26.6, introduced per task | Each shipped capability has the matching just/cargo check. |
| HLCOV-OPS-008 | §27.1 | T37, T40 | topology/runbook review; `just full-coverage-soak` | Initial topology matches §27.1. |
| HLCOV-OPS-009 | §28.19 | T38, T40 | `just full-coverage-soak`; chaos/restore gates | Expansion is not complete without those gates. |
| HLCOV-OPS-010 | T01 | T01 | `just hyperliquid-full-coverage-docs` | Renamed spec/plan, IDs, traceability, plans README, ROADMAP, and 2026-08-20 STATUS snapshot. |
| HLCOV-OPS-011 | T35, T39 | T35, T39 | coverage check; release profile fixture | First release enables no third-party providers. |
| HLCOV-OPS-012 | T36, T37 | T36, T37 | out-of-repo T36 gate; `just` observability checks | T36 is not implemented in alpha-desk. T37 may ship without it. |
