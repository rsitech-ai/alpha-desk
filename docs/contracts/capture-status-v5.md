# Capture Status V5

`hl.capture.status.v5` is the atomic, bounded, machine-readable operator
snapshot written by `hl-capture run` and returned by `hl-capture status
--json` after a current writer. It extends frozen V4 with a required
`maintenance` object. It contains no payloads, filesystem paths, connection
strings, credentials, or private infrastructure identity.

This is a writer-schema change. It is not Stage 1 PASS, soak evidence, or
live-source qualification.

V5 contains every V4 field with the same meanings. See
[`capture-status-v4.md`](capture-status-v4.md).

## Maintenance

`maintenance` is required on V5 and omitted on V4. Inactive capture, including
runtimes that do not run packing or GC, still writes V5 with a fail-closed
idle object:

- `enabled: false` and `kill_switch: false` unless a maintenance task is
  actually configured and the operator kill-switch file is latched;
- `health: green` with `reason_code` omitted, or yellow/red with a reason;
- archive statistic counters at zero until a maintenance cycle reports them;
- optional `last_scrub_at_micros`, `last_pack_index_at_micros`,
  `last_pack_data_at_micros`, and `last_retention_at_micros` omitted until
  that work succeeds; and
- `retention_authorized: false` unless a verified backup-receipt artifact
  currently authorizes packed-object retention. A configured hex token is not
  enough.

A green maintenance object omits `reason_code`. Yellow or red always carries
one. Readers must not treat `enabled: false` or unauthorized retention as a
qualification or Stage PASS claim.

## Last-heartbeat rates

`throughput_records_per_sec` and `throughput_blocks_per_sec` are last-heartbeat
windowed observations. They are omitted when no heartbeat window has been
sampled. Missing rates stay omitted; they are not invented as zero. An explicit
zero is a sampled idle window, not live-qualification.

## Compatibility

Readers still accept frozen `hl.capture.status.v4` snapshots that omit
`maintenance`. A V4 snapshot that smuggles `maintenance`, a V5 snapshot
without `maintenance`, or an unknown schema id fail closed. A V5-only reader
must not treat a ready V4 snapshot as live-ready. Production `GET /healthz`
reports ready only for V5 with fail-closed `maintenance` present; a leftover
ready V4 snapshot is not-ready (503), not `ready: true`. `GET /status` may
still return V4 bytes as-read.

Archive, spool, and publication formats are independently versioned. A process
starting over a V4, malformed, or foreign-build snapshot writes a fresh V5
snapshot for its own build and chain rather than trusting stale status state.
