# Hyperliquid Alpha Desk

Hyperliquid Alpha Desk is a local-first, read-only market-intelligence and research workstation under active development by RSI Tech. Its production design centers on byte-preserving source capture, a deterministic canonical ledger, reproducible research, evidence-linked signals, and native Apple clients.

This repository is not yet a runnable desk application. It currently contains a substantial Stage 0 engineering foundation and the approved staged design. The Stage 0 release gate remains on `HOLD`; the capture runtime, APIs, research workflows, and native UI are planned work.

## Current state

| Area | Status | Evidence |
| --- | --- | --- |
| Production design and staged plans | Approved | [`docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md`](docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md) |
| Rust workspace, exact domain types, schemas, fixtures, telemetry, and provenance | Implemented and locally tested | [`docs/STATUS.md`](docs/STATUS.md) |
| Stage 0 gate tooling | Implemented; gate outcome `HOLD` | [`config/stage-gates/stage-0.toml`](config/stage-gates/stage-0.toml) |
| Dependency stack | Defined for local development; runtime smoke still required for each release candidate | [`infra/docker-compose/README.md`](infra/docker-compose/README.md) |
| Source-observation and capture configuration contracts | Implemented on the hardening branch; no source adapter or runtime yet | [`docs/STATUS.md`](docs/STATUS.md) |
| Durable capture and canonical truth-layer runtime | Not implemented | [`docs/ROADMAP.md`](docs/ROADMAP.md) |
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
```

`just verify` checks the workspace shape, formatting, clippy, architecture boundaries, dependency policy, Rust tests, and Swift tests. It does not start a product runtime.

To validate the local dependency stack separately:

```sh
just stage-0-compose-smoke
```

For focused commands and development conventions, read [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Repository map

- `crates/` — domain contracts, canonical types, storage ports, telemetry, and research foundations
- `services/` — future long-running service boundaries; currently bootstrap-only
- `apps/AlphaDesk/` — Swift package foundations; currently no application target
- `schemas/` — versioned Protobuf and JSON contracts
- `fixtures/` — synthetic, redistributable deterministic fixtures
- `infra/` — local dependency and future deployment scaffolding
- `tools/` — schema, architecture, provenance, fixture, and stage-gate tooling
- `docs/superpowers/` — approved design, stage plans, traceability, and reviews

## Safety boundary

V1 is read-only by design. This workspace contains no trading signer, exchange private-key handling, order-placement route, custodial function, or live execution service. Simulation code does not grant execution capability. Any future execution enclave requires a separate design, threat model, approval, and repository boundary.

Research outputs can be incomplete, delayed, provisional, statistically weak, or wrong. They are not a promise of profitability and are not financial advice. Hyperliquid Alpha Desk is an independent project and is not affiliated with, endorsed by, or sponsored by Hyperliquid.

## Contributing and support

Start with [CONTRIBUTING.md](CONTRIBUTING.md), the [roadmap](docs/ROADMAP.md), and the [current status ledger](docs/STATUS.md). Use [SUPPORT.md](SUPPORT.md) for ordinary help and [SECURITY.md](SECURITY.md) for sensitive vulnerability reports. Do not put credentials, private wallet labels, proprietary feed data, private alpha, or internal deployment details in issues or pull requests.

## License

The current repository is licensed under [Apache License 2.0](LICENSE). The approved design records a possible future dual-license choice subject to legal review; no such change has been approved.
