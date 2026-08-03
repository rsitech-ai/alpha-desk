# Hyperliquid Alpha Desk

Hyperliquid Alpha Desk is a local-first, read-only market-intelligence and research workstation under active development by RSI Tech. Its production design centers on byte-preserving source capture, a deterministic canonical ledger, reproducible research, evidence-linked signals, and native Apple clients.

This repository is not yet a complete desk application. It contains a runnable
read-only capture service for the currently qualified empty committed-block
mapping, plus substantial Stage 0 foundations, canonical identity, continuity,
durable publication, immutable archive work, deterministic watermark-only
state, exact synthetic canonical trade-fact, order-lifecycle, and
market-registry reducers, private local checkpoints, and synthetic replay
evidence runners. The Stage 0 release gate remains on `HOLD`;
action-bearing source mapping and full account/order state, production hot-state
storage, APIs, research workflows, and native UI remain incomplete.

## Current state

| Area | Status | Evidence |
| --- | --- | --- |
| Production design and staged plans | Approved | [`docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md`](docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md) |
| Rust workspace, exact domain types, schemas, fixtures, telemetry, and provenance | Implemented and locally tested | [`docs/STATUS.md`](docs/STATUS.md) |
| Stage 0 gate tooling | Implemented; gate outcome `HOLD` | [`config/stage-gates/stage-0.toml`](config/stage-gates/stage-0.toml) |
| Dependency stack | Defined for local development; runtime smoke still required for each release candidate | [`infra/docker-compose/README.md`](infra/docker-compose/README.md) |
| Source observation, strict recoverable spool, primary-node adapter, source-trust admission, conservative committed mapper, and canonical sequencing | Runnable and locally restart/soak tested with synthetic empty node-format blocks; not live-source qualified | [`docs/STATUS.md`](docs/STATUS.md) |
| Immutable canonical/raw Parquet archive, verified compaction, replay reads, and offline inspection | Canonical blocks and verified closed-spool raw observations are wired; compaction remains an offline operation | [`docs/formats/archive-manifest-v1.md`](docs/formats/archive-manifest-v1.md) |
| Deterministic state, local checkpoints, and serial replay | Watermark-only operation is runnable; exact synthetic canonical trade facts, stored quantity-symmetry assessments, order lifecycle, and market registry are reducer/replay tested; full action state and production RocksDB remain unqualified | [`docs/contracts/deterministic-state-v1.md`](docs/contracts/deterministic-state-v1.md) |
| Complete durable capture and canonical truth-layer runtime | Partially implemented; action mappings, real independent-source qualification, overlap reconciliation, and production replay service remain | [`docs/ROADMAP.md`](docs/ROADMAP.md) |
| Long-running services, REST/WebSocket API, macOS/iOS apps | Not implemented | [`docs/STATUS.md`](docs/STATUS.md) |
| Public OSS release | Prepare-only; blocked by export, legal, history, runtime, and external publication gates | [`docs/RELEASE.md`](docs/RELEASE.md) |

No part of this table should be read as evidence of trading performance, complete venue coverage, production deployment, or release readiness.

## Architecture

The approved system is intentionally evidence-first:

```text
primary and independent sources
        │
        ▼
byte-preserving observation spool
        │
        ▼
canonicalization and continuity checks
        │
        ├── immutable archive and deterministic replay
        └── operational event streams
                    │
                    ▼
state, intelligence, research, APIs, and native clients
```

Public WebSocket data is a provisional or reconciliation source, not a substitute for committed primary evidence. Canonical publication occurs only after durability and continuity policy pass. See the [architecture overview](docs/architecture/overview.md) and [approved design](docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md).

## Local verification

Prerequisites:

- macOS or Linux for the Rust workspace
- Rust and Cargo `1.97.1` via the pinned toolchain
- `just`
- Swift `6.3` for the Apple package
- Docker with Compose for dependency-stack smoke checks

Run the normal local verification:

```sh
just verify
just generated
just spool-verify
cargo +1.97.1 test -p hl-analytics --test archive --locked --offline
cargo +1.97.1 test -p archive-inspect --locked --offline
just state-replay-e2e
just state-replay-trade-e2e
just state-replay-order-e2e
just state-replay-market-e2e
just state-replay-position-e2e
```

`just verify` checks the workspace shape, formatting, clippy, architecture boundaries, dependency policy, Rust tests, and Swift tests. It does not start a product runtime.

The committed synthetic spool fixture can be inspected independently with `just spool-verify`.
Parser fuzzing additionally requires the pinned `nightly-2026-07-16` toolchain and
`cargo-fuzz`; run `just spool-fuzz`.
`just state-replay-e2e` runs the bounded synthetic Stage 2 evidence path; use
`just state-replay-soak` for the longer repeated rebuild profile. Neither
qualifies live source semantics or Stage 2.
`just state-replay-trade-e2e` exercises the exact canonical trade reducer,
decoded fact/participant/reconciliation cardinality, checkpoint resume, and
atomic malformed/unsupported rejection. Use `just state-replay-trade-soak` for
the longer bounded profile. These commands use generated canonical events:
they do not qualify Stage 1, Stage 2, deployed source semantics, or
account/order/position state.
`just state-replay-order-e2e` exercises all seven exact canonical order event
contracts, immutable facts, current lifecycle state, hash-linked transition
assessments, checkpoint resume, and atomic malformed/unsupported rejection.
Use `just state-replay-order-soak` for the longer bounded profile. The report
proves only the generated canonical order contract: Stage 1/2, deployed/live
source, position, margin, and execution qualification remain false.
`just state-replay-market-e2e` exercises all twelve exact canonical market
event contracts in prerequisite order, strictly decodes every registry
namespace, proves repeated and checkpoint-resumed state equality, and verifies
hash-only metadata suppression plus atomic invalid/unsupported rejection. Use
`just state-replay-market-soak` for the longer bounded release profile. The
report proves only the generated canonical market contract: Stage 1/2,
deployed/live source, authoritative metadata, external oracle reconciliation,
account, position, margin, book, signal, and execution qualification remain
false.
`just state-replay-account-e2e` exercises generated canonical account flows,
relations, and mode changes through the immutable archive and private composite
checkpoint path. Use `just state-replay-account-soak` for the bounded longer
profile. It proves only `synthetic_account_flow_contract_proven`; it does not
qualify positions, episodes, liquidations, settlement, funding attribution,
margin models, authoritative balances, deployed/live source, Stage 1/2, book,
signal, or execution semantics.
`just state-replay-position-e2e` retains the separate synthetic canonical
position proof: an enriched opening checkpoint followed by reversal, funding,
liquidation, backstop interruption, settlement, and exact source re-anchor.
It checks repeated full replay, segmented checkpoint resume, literal decoded
position/episode/liquidation state, a settlement-PnL semantic mutation, and
three atomic rejection boundaries. Use `just state-replay-position-soak` for
the longer bounded profile. This does not qualify deployed/live source,
authoritative positions or balances, venue reconciliation, protocol entry
price, source closed PnL, fees, TWAP, backstop basis, margin, liquidation
price, Stage 1/2, book, signal, execution, or a live product.
For an existing canonical archive, use `just state-replay-archive-e2e` or
`just state-replay-archive-soak` with an explicit chain, inclusive range, and
manifest-boundary checkpoint height. Operator-archive evidence remains
watermark-only and unqualified.

To validate the local dependency stack separately:

```sh
just stage-0-compose-smoke
```

For focused commands and development conventions, read [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Repository map

- `crates/` — domain contracts, stable canonical event/block identity, storage ports, telemetry, and research foundations
- `services/` — service boundaries; `hl-capture` contains the runnable
  raw-first primary-node/spool/canonical-publication path
- `apps/AlphaDesk/` — Swift package foundations; currently no application target
- `schemas/` — versioned Protobuf and JSON contracts
- `fixtures/` — deterministic synthetic fixtures and provenance-labeled normalized public schema examples
- `infra/` — local dependency and future deployment scaffolding
- `tools/` — schema, architecture, provenance, fixture, spool/archive
  inspection, state replay evidence, and stage-gate tooling
- `docs/superpowers/` — approved design, stage plans, traceability, and reviews

## Safety boundary

V1 is read-only by design. This workspace contains no trading signer, exchange private-key handling, order-placement route, custodial function, or live execution service. Simulation code does not grant execution capability. Any future execution enclave requires a separate design, threat model, approval, and repository boundary.

Research outputs can be incomplete, delayed, provisional, statistically weak, or wrong. They are not a promise of profitability and are not financial advice. Hyperliquid Alpha Desk is an independent project and is not affiliated with, endorsed by, or sponsored by Hyperliquid.

## Contributing and support

Start with [CONTRIBUTING.md](CONTRIBUTING.md), the [roadmap](docs/ROADMAP.md), and the [current status ledger](docs/STATUS.md). Use [SUPPORT.md](SUPPORT.md) for ordinary help and [SECURITY.md](SECURITY.md) for sensitive vulnerability reports. Do not put credentials, private wallet labels, proprietary feed data, private alpha, or internal deployment details in issues or pull requests.

## License

The current repository is licensed under [Apache License 2.0](LICENSE). The approved design records a possible future dual-license choice subject to legal review; no such change has been approved.
