# Hyperliquid Alpha Desk — Approved Design and Implementation Plans

This repository contains the approved production design and the complete staged implementation plan for a private, local-only Hyperliquid market-intelligence and alpha-research desk.

The design defines the production architecture, canonical data model, deterministic state reconstruction, wallet/entity intelligence, market-sentiment framework, signal validation, native SwiftUI desk, security boundaries, operations, testing, and phased acceptance gates. The implementation plan translates that design into reviewer-sized, test-driven tasks with exact files, interfaces, commands, expected results, commits, and stage gates.

## Canonical documents

- [Approved production design](docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md)
- [Implementation-plan index](docs/superpowers/plans/README.md)
- [Program roadmap](docs/superpowers/plans/2026-07-24-00-hyperliquid-alpha-desk-program-roadmap.md)
- [Specification traceability](docs/superpowers/plans/2026-07-24-99-spec-traceability.md)
- [Plan self-review](docs/superpowers/plans/2026-07-24-98-plan-self-review.md)

## Selected baseline

- Rust 1.97.1, edition 2024, for the canonical event-sourced core, replay, research, APIs, and tooling.
- Swift 6.3, SwiftUI, Swift Charts, GRDB, and Core ML for the native Apple desk and local personalization.
- NATS JetStream, RocksDB, ClickHouse LTS, PostgreSQL, Arrow/Parquet, DataFusion, Polars, and ONNX Runtime.
- Kanidm for self-hosted OIDC/WebAuthn.
- Dedicated Ubuntu 24.04/systemd hot path with Ansible/Podman for reproducible local deployment; no mandatory Kubernetes.
- Read-only V1 with no trading signer, exchange private key, or order-placement path.

## Status

Design version 1.0.0 was approved for implementation on 2026-07-24. The implementation-plan set is complete. The Stage 0 workspace bootstrap is in place; production domain behavior proceeds only through evidence-based stage gates.

The future execution enclave is outside V1 and requires a separate threat model, approved design, and implementation plan after shadow-live and paper evidence satisfy the admission policy.

## Stage 0 gate

The committed Stage 0 contract is
[`config/stage-gates/stage-0.toml`](config/stage-gates/stage-0.toml). Run it
only from the clean, frozen implementation commit:

```sh
just stage-0-gate
```

The command writes transient canonical JSON only to the Git-ignored
`target/stage-gates/stage-0.json` and writes the exact canonical local builder
evidence to `target/stage-gates/stage-0.builder.json`. Copy Builder B's
`stage-0.builder.json` byte-for-byte to Builder A's configured
`target/stage-gates/inputs/stage-0.builder-b.json`; no JSON extraction or
rewriting is required. Exit status `0` means `PASS`, `1` means a local
verification `FAIL`, and `2` means `BLOCKED`. A local builder remains
`BLOCKED` until a second independent builder report, the exact required GitHub
check proof, two distinct detached reviewer approvals, a configured reviewer
keyring, and usable OpenPGP verification tooling are supplied. Any non-PASS
result has the explicit stage outcome `HOLD`. External reports, proofs,
signatures, and the keyring stay under the ignored input paths named by the
configuration. The gate never creates an approval record, signature, evidence
commit, or tag.

The tracked operational trust registry is
[`stage-0-trust-policy.toml`](config/stage-gates/stage-0-trust-policy.toml).
Its current placeholder fingerprints intentionally keep Stage 0 blocked. They
must be replaced by distinct, reviewed, full fingerprints in a committed
change; the gate hashes the exact committed registry bytes. The separate
[`stage-0-trust-policy.example.toml`](config/stage-gates/stage-0-trust-policy.example.toml)
remains a non-operational template for the `platform-data` and `independent`
roles.

## V1 safety boundary

The current V1 is read-only. It contains no execution service, trading signer, exchange private-key handling, order-placement path, or signing capability. Any future execution enclave is explicitly outside this workspace boundary until separately designed, reviewed, and approved.
