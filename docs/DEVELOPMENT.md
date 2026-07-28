# Development Guide

## Toolchains

- Rust/Cargo: `1.97.1`, pinned by `rust-toolchain.toml`
- Rust edition: 2024
- Swift: 6.3
- Docker Compose: required only for dependency-stack and integration smokes
- Parser fuzzing: `nightly-2026-07-16` plus `cargo-fuzz 0.13.2`

Use the committed lockfiles. Normal verification is offline after dependencies have been fetched.

## Start here

```sh
just --list
just verify
just generated
just spool-verify
SOURCE_DATE_EPOCH=1784894400 just reproducible
```

The reproducibility check intentionally requires an explicit unsigned
`SOURCE_DATE_EPOCH`; it does not infer a timestamp from the working tree or
ambient clock.

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

The focused durable-spool checks are:

```sh
cargo +1.97.1 test -p hl-capture --test spool_recovery --locked --offline
cargo +1.97.1 test -p spool-inspect --locked --offline
just spool-verify
just spool-fuzz
```

`just spool-fuzz` runs for 60 seconds by default. It validates parser safety, not service uptime or
source completeness. The normative framing and recovery contract is
[`formats/spool-v1.md`](formats/spool-v1.md).

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
