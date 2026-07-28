# Development Guide

## Toolchains

- Rust/Cargo: `1.97.1`, pinned by `rust-toolchain.toml`
- Rust edition: 2024
- Swift: 6.3
- Docker Compose: required only for dependency-stack and integration smokes

Use the committed lockfiles. Normal verification is offline after dependencies have been fetched.

## Start here

```sh
just --list
just verify
just generated
```

Focused Rust work should use the smallest package:

```sh
cargo +1.97.1 test -p <package> --locked --offline
cargo +1.97.1 clippy -p <package> --all-targets --locked --offline -- -D warnings
```

The full local dependency-stack smoke is:

```sh
just stage-0-compose-smoke
```

`just dev-up` starts PostgreSQL, NATS, ClickHouse, MinIO, and related development dependencies. It does not start Alpha Desk services or a UI.

## Engineering rules

- Write a focused failing test before behavior changes.
- Keep deterministic domain logic synchronous and separate from adapters.
- Validate all boundary input and reject unknown configuration keys.
- Preserve source bytes and explicit version/provenance fields.
- Make retry, quarantine, stop, and recovery behavior typed and observable.
- Use bounded queues, payloads, logs, timeouts, and shutdown paths.
- Do not add execution, signer, credential, or order-placement capability to V1.
- Do not weaken a gate to make a change pass.

## Stage plans

The detailed implementation plans live under `docs/superpowers/plans/`. They are approved design inputs and retain their original checklist state. Current implementation evidence is recorded in `docs/STATUS.md`.

Stage 1 normally requires a verified signed `stage-0-foundations` tag. Work developed before that external gate closes must remain clearly labeled as unreleased development and cannot be used to claim the gate passed.

## Test boundaries

- Unit tests prove pure invariants and error semantics.
- Boundary integration tests prove serialization, storage, process, and dependency contracts.
- Runtime smokes prove real startup, readiness, shutdown, listener release, and owned-resource cleanup.
- Long-running evidence becomes meaningful only after durable spool recovery, continuity, archive, replay, and source-health contracts exist.

## Sensitive material

Never commit credentials, private keys, real wallet labels, proprietary operator-feed fixtures, private alpha thresholds/results, model bundles, production inventory, certificates, or internal hostnames. Use synthetic or explicitly redistributable fixtures. Report suspected exposures through [SECURITY.md](../SECURITY.md), not a public issue.
