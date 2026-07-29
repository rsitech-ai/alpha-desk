# Changelog

All notable changes will be documented in this file. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and released versions will use semantic versioning where the public contracts make that meaningful.

## [Unreleased]

### Added

- Approved production design and staged implementation plans.
- Rust 1.97.1 workspace with checked fixed-point domain types and identifiers.
- Versioned Protobuf contracts and deterministic synthetic fixtures.
- Telemetry, build provenance, architecture, dependency, generated-artifact, reproducibility, and Stage 0 gate tooling.
- Local dependency and inactive deployment scaffolding.
- Strict source-observation, cursor, source-error, async source-port, and capture-configuration contracts for Stage 1.
- Crash-safe capture spool with durability receipts, recovery scanning, hash-chained close
  manifests, offline inspection, deterministic fixtures, and parser fuzzing.
- Primary-node per-height and newline-file adapters with stable restart cursors, explicit durable
  acknowledgement, rotation/truncation handling, gap detection, and byte-preserving quarantine.
- Exhaustive source-trust admission policy that keeps public, provisional,
  recovery, snapshot, auxiliary, and mempool evidence out of the committed
  watermark lane.
- Source-independent V1 event IDs, validated production canonical-event
  construction, deterministic canonical block hashes, and fail-closed block
  ordering/boundary invariants.
- Fail-closed OSS classification and content audit with seeded leak canaries.
- Contributor, governance, support, security, architecture, status, and release documentation.

### Security

- V1 workspace boundary excludes trading signers, exchange private-key handling, order placement, and custodial capability.

### Known limitations

- Stage 0 gate remains on `HOLD`.
- Independent/recovery transports, proprietary operator-feed integration,
  real-node corpus qualification, the long-running capture runtime,
  exhaustive source-to-canonical mapping, continuity, archive publication,
  APIs, research workflows, and native applications are not yet implemented.
- No public release, supported version, production deployment, or performance qualification exists.
