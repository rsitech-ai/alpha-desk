# Hyperliquid Alpha Desk — Design Specification

This repository contains the owner-review design for a private, local-only Hyperliquid market-intelligence and alpha-research desk.

The specification defines the production architecture, canonical data model, deterministic state reconstruction, wallet/entity intelligence, market-sentiment framework, signal validation, native SwiftUI desk, security boundaries, operations, testing, and phased acceptance gates.

## Primary document

[`docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md`](docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md)

## Selected baseline

- Rust 2024 canonical/event-sourced core.
- Swift 6.3, SwiftUI, Swift Charts, GRDB, and Core ML for the Apple desk.
- NATS JetStream, RocksDB, ClickHouse LTS, PostgreSQL, Arrow/Parquet, DataFusion, and ONNX Runtime.
- Kanidm for self-hosted OIDC/WebAuthn.
- Dedicated Ubuntu/systemd hot path; no mandatory Kubernetes.
- Read-only V1 with no trading signer.

## Status

Design draft for owner review. Implementation planning begins only after the design is approved.
