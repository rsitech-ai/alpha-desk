# Raw Archive and Disk Safety Implementation Plan

**Goal:** Make the production capture entrypoint retain every durably spooled
observation in the immutable raw Parquet archive and stop before writes consume
the configured filesystem reserve.

**Authority boundary:** This is local, read-only data acquisition. It does not
add trading, signing, private-key access, remote publication, live-source
qualification, or speculative action mappings.

**Architecture:** The spool remains the first acknowledgement boundary. Closed,
hash-verified spool segments become the batching boundary for raw Parquet
objects because only a closed segment has stable segment and manifest hashes.
Startup replays every verified closed segment idempotently, seals a recovered
non-empty tail, archives it, and only then resumes the source. Rotation and
graceful shutdown archive their newly closed segment before the task exits.
Canonical mapping still reads the same durable spool records, so raw archival
and interpretation cannot diverge through separate parsers.

Disk safety is a small injected policy around filesystem statistics. The
runtime checks both spool and archive filesystems before each source write,
uses checked arithmetic for the configured reserve plus the anticipated
payload, and fails with a stable reason code if either filesystem cannot
preserve the reserve. The check is advisory to the subsequent atomic/fsynced
write—filesystem write errors remain independently fatal.

## Slice 1: Closed-segment archival contract

- Expose verified closed-segment receipts with exact segment and manifest
  hashes and paths.
- Return rotation receipts from `SourceSpool::append`.
- Allow startup to seal a recovered non-empty tail while keeping one new active
  segment.
- Convert verified segment records back into byte-identical
  `SourceObservation` values and group them by hour for raw archive batches.
- Archive each batch idempotently and verify the returned manifest.
- Test restart replay, rotation, shutdown, hour-boundary splitting, and
  conflicting raw ranges.

## Slice 2: Runtime ordering

- Keep the concrete archive available through both canonical and raw archive
  ports.
- Archive all closed segments before source resume.
- Archive newly closed segments after rotation and on graceful shutdown.
- Fail the owned source task if raw append or verification fails.
- Extend the production-entrypoint E2E to require raw observation parity with
  spool and canonical block counts after restart.

## Slice 3: Disk reserve

- Add an injectable disk-space probe and pure checked reserve decision.
- Check spool and archive roots before accepting each next source observation.
- Fail closed on probe errors, arithmetic overflow, or insufficient available
  bytes; do not acknowledge the source record.
- Add focused tests for exact-boundary success, one-byte-short failure,
  overflow, and probe failure.
- Surface only stable non-secret reason codes.

## Completion checks

- Focused red-green tests for spool receipts, raw archival, and disk policy.
- Rustfmt and strict Clippy for touched targets.
- Full `hl-capture`, archive, and workspace tests.
- Fresh source E2E and short soak showing raw/spool/block/publication parity.
- Archive and spool inspection, secret scan, OSS audit, and documentation
  updates that retain `live_source_qualified=false`.
