# Capture Status V3

> Frozen compatibility contract. Current runtimes publish V4; see
> [`capture-status-v4.md`](capture-status-v4.md).

`hl.capture.status.v3` is the atomic, machine-readable operator snapshot written
by `hl-capture run` and returned by `hl-capture status --json`. It contains no
source payloads, filesystem paths, connection strings, credentials, or private
infrastructure identity.

The fields are:

- `snapshot_at_micros`, `build_id`, and `chain_id`: snapshot provenance;
- `health`, `ready`, and optional `last_error_reason`: current service gate;
- `active_committed_source`: either `locally-verified-committed` or
  `independent-committed`;
- `primary_source_health` and optional `independent_source_health`: bounded
  source state (`starting`, `healthy`, or `range-unavailable`) without source
  IDs, paths, or operator identity;
- optional `failover_height` and `failover_reason`: present together exactly
  when the independent source is active;
- optional `durable_height`: highest archive-, journal-, publication-, and
  cursor-complete canonical height;
- `pending_blocks`: archived publication plans still pending completion;
- `capture_backlog_records`: fsynced source records from the oldest unapplied
  capture height through the latest captured height;
- optional `oldest_pending_capture_height`: present exactly when the capture
  backlog is non-zero;
- optional `disk_free_basis_points`: the lowest available-space percentage
  across the spool and archive filesystems, expressed in basis points; and
- optional `archive_manifest_id`: the manifest bound to the durable height.

`pending_blocks` and `capture_backlog_records` are intentionally distinct. The
first describes partially completed downstream publication work. The second
describes fsynced evidence from the active committed source not yet reflected
in the durable canonical cursor. Retained standby evidence is not included in
this active-source backlog.

## Failover policy

- Primary operation may be green when every other health invariant permits it.
- Independent operation is always yellow. It may be ready only after the
  create-once V1 failover decision is durable and the exact failover height is
  present in the independent spool.
- Recovery and status-heartbeat tasks cannot promote an active independent
  source to green.
- A primary gap without exact independent evidence, or a gap in the active
  independent source, is red and non-ready.
- V3 never exposes source IDs, filesystem paths, or infrastructure identity.

## Disk policy

- At or above 20% free (`2000` basis points), disk health may be green.
- From 10% inclusive to 20% exclusive, the runtime remains ready but reports
  yellow with `capture_disk.low_space`.
- Below 10%, new writes fail closed with
  `capture_disk.insufficient_free_percentage`.
- The configured absolute reserve is enforced independently. Passing the
  percentage gate never permits a write which would consume that reserve.

The percentage and backlog values are updated from the owned acquisition and
drain loops. A missing disk value means capacity has not yet been measured; it
must not be interpreted as healthy.

## Compatibility

V3 replaces V2 because source selection and failover state change the
operational meaning of readiness and backlog. The status file is an ephemeral
snapshot, not a durable database. A process starting over an older or malformed
snapshot writes a fresh V3 snapshot for its own build and chain rather than
trusting stale values.
