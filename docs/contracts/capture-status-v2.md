# Capture Status V2

`hl.capture.status.v2` is the atomic, machine-readable operator snapshot written
by `hl-capture run` and returned by `hl-capture status --json`. It contains no
source payloads, filesystem paths, connection strings, credentials, or private
infrastructure identity.

The fields are:

- `snapshot_at_micros`, `build_id`, and `chain_id`: snapshot provenance;
- `health`, `ready`, and optional `last_error_reason`: current service gate;
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
describes raw source evidence not yet reflected in the durable canonical
cursor.

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

V2 replaces V1 because the backlog and disk fields change the operational
meaning of readiness. The status file is an ephemeral snapshot, not a durable
database. A process starting over an older or malformed snapshot writes a fresh
V2 snapshot for its own build and chain rather than trusting stale values.
