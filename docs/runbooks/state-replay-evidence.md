# State Replay Evidence

`state-replay fixture-e2e` is the first runnable Stage 2 evidence path. It is a
bounded synthetic fixture runner, not a live-source or production-state
qualification.

## Quick evidence run

From the repository root:

```bash
just state-replay-e2e
```

The recipe creates a new private directory under
`target/evidence/state-replay/` and prints the resulting `report.json` path. It
never reuses or overwrites an existing evidence directory.

The run:

1. creates a deterministic local canonical Parquet archive;
2. rebuilds its empty committed-block range independently for the requested
   number of iterations;
3. requires every final state hash and full replay receipt hash to match;
4. replays a prefix, publishes a private immutable local checkpoint, reloads
   it under exact archive/schema/reducer compatibility, and resumes the suffix;
5. requires resumed and uninterrupted state hashes to match; and
6. appends one typed trade block and requires the watermark-only production
   reducer to quarantine it without advancing state.

The report explicitly records:

- `evidence_class = "synthetic_fixture"`;
- `stage_2_qualified = false`;
- `live_source_qualified = false`;
- deterministic full and resumed state/receipt hashes;
- the content-derived checkpoint ID;
- iteration count and replay duration; and
- poison-block reason, progress, and before/after state hashes.

## Longer soak

```bash
just state-replay-soak
```

Defaults are 1,000 archived blocks, a checkpoint after 500 blocks, and 100
independent full replays. Override the bounded parameters when needed:

```bash
just state-replay-soak 2000 1000 500
```

The runner rejects fewer than two blocks, a checkpoint outside the range, zero
iterations, more than 100,000 blocks or iterations, or more than 100,000,000
total replayed blocks. Disk use is driven mainly by the one-time Parquet
archive; runtime grows approximately with `blocks * iterations`.

## Interpreting the result

A successful report is `runtime-proven:synthetic` evidence for deterministic
serial replay, local checkpoint resume, and poison-block atomicity. It does not
prove action-bearing reducers, RocksDB durability, deployed Hyperliquid source
compatibility, reconciliation, service readiness, or the Stage 2 gate.

Retain the complete evidence directory when comparing runs. The archive,
checkpoint generation, and report belong together; do not copy only the JSON
and call it reproducible evidence.
