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

Design version 1.0.0 was approved for implementation on 2026-07-24. The implementation-plan set is complete. Production code has not yet been implemented; execution begins with Stage 0 Foundations and proceeds only through evidence-based stage gates.

The future execution enclave is outside V1 and requires a separate threat model, approved design, and implementation plan after shadow-live and paper evidence satisfy the admission policy.
