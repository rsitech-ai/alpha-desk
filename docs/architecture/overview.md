# Architecture Overview

Hyperliquid Alpha Desk is designed as an evidence-first, event-sourced research system. The source evidence and canonical ledger are the authority; queues, caches, APIs, and clients are derived views.

## Data flow

1. Primary and independent source adapters produce byte-preserving observations with explicit source identity, version, cursor, timestamps, parser version, warnings, and content hash.
2. Capture acknowledges progress only after the observation is durable in a local append-only spool.
3. Deterministic canonicalization converts retained evidence into versioned block and event envelopes.
4. A sequencer enforces continuity. Gaps, unknown variants, and conflicting stable identities are quarantined and alarmed rather than silently skipped.
5. Canonical data is durably archived before it is published to operational streams.
6. State reducers, intelligence, signals, research, APIs, and clients consume reproducible versioned contracts.

## Trust classes

- Primary committed evidence is the main canonical input.
- Independent secondary evidence detects omissions and divergence.
- Historical sources repair approved ranges but do not silently override live evidence.
- Public market feeds and mempool-like observations are provisional.
- Operator feeds may be proprietary. Public code exposes the adapter boundary, not private schemas, fixtures, credentials, or source-specific operational details.

## Determinism and correctness

- Financial values use checked fixed-point representations.
- Protocol order is preferred over ingestion time.
- Stable event identity plus a changed content hash is a critical divergence.
- Unknown source variants fail closed into quarantine.
- NATS delivery is at least once; downstream messages carry deduplication identity.
- Replay must reproduce the same canonical state from the same evidence and versions.

## Process boundaries

- `hl-capture` owns asynchronous source I/O, spooling, source health, and canonical publication.
- `hl-core` owns deterministic canonical state reconstruction.
- `hl-analytics` owns evidence-linked feature and intelligence computation.
- `hl-research` owns reproducible experiment and model evaluation.
- `hl-api` exposes read-only versioned contracts to clients.
- The Apple client presents evidence and health; it never becomes a second canonical-computation engine.

Most of these processes are planned rather than operational. See [Implementation Status](../STATUS.md) for current evidence and the [approved design](../superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md) for the complete target.
