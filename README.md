# Hyperliquid Alpha Desk

Hyperliquid Alpha Desk is a local-first, read-only market-intelligence and
research workstation maintained by [RSI Tech](https://rsitech.ai).

The long-term design is byte-preserving source capture, a deterministic
canonical ledger, reproducible research, evidence-linked signals, and native
Apple clients. This repository is not yet that complete desk. It currently
ships a runnable read-only capture path, Stage 0 foundations, canonical
identity and continuity work, archive and replay tooling, and synthetic
reducer evidence. Native UI, production APIs, and live-source qualification
are incomplete.

> Status: public preview. The Stage 0 release gate remains on hold. This is
> not a trading product, not financial advice, and not affiliated with
> Hyperliquid.

## What is in the tree

- `crates/` — domain contracts, canonical identity, storage ports, telemetry
- `services/` — service boundaries, including the `hl-capture` runtime
- `apps/AlphaDesk/` — Swift package foundations (no application target yet)
- `schemas/` — versioned Protobuf and JSON contracts
- `fixtures/` — deterministic synthetic fixtures
- `infra/` — local dependency scaffolding
- `tools/` — schema, archive, replay, and stage-gate tooling
- `docs/STATUS.md` — current implementation ledger
- `docs/architecture/overview.md` — architecture overview

V1 is read-only. The workspace has no trading signer, exchange private-key
handling, order-placement route, or custodial function.

## Requirements

- macOS or Linux
- Rust `1.97.1` from `rust-toolchain.toml`
- `just`
- Swift 6.3 for the Apple package
- Docker with Compose for optional dependency-stack smoke checks

## Local verification

```sh
git clone https://github.com/rsitech-ai/alpha-desk.git
cd alpha-desk
just verify
just generated
just spool-verify
```

`just verify` checks workspace shape, formatting, Clippy, architecture
boundaries, dependency policy, Rust tests, and Swift tests. It does not start
a product runtime.

Optional synthetic replay evidence:

```sh
just state-replay-e2e
just state-replay-trade-e2e
just state-replay-order-e2e
just state-replay-market-e2e
just state-replay-position-e2e
```

These commands prove generated canonical contracts only. They do not qualify
live source semantics, Stage 1/2, or a production desk.

Watch capture status without PostgreSQL or NATS:

```sh
cargo +1.97.1 run -p hl-capture --locked --offline -- \
  serve-status --config config/capture.example.toml --listen 127.0.0.1:8741
```

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for focused commands.

## Contributing and support

[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) ·
[Support](SUPPORT.md) · [Code of Conduct](CODE_OF_CONDUCT.md) ·
[Roadmap](docs/ROADMAP.md) · [Status](docs/STATUS.md)

Do not put credentials, private wallet labels, proprietary feed data, or
internal deployment details in issues or pull requests.

Public and confidential contact: [info@rsitech.ai](mailto:info@rsitech.ai)

## License

[Apache License 2.0](LICENSE)
