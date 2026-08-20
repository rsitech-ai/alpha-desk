# Capture Status V5

`hl.capture.status.v5` extends the frozen V4 operator snapshot with optional
V3 archive maintenance visibility. Capture writes V5 only when a maintenance
object is present. Snapshots with no maintenance stay on frozen
`hl.capture.status.v4`, so older `deny_unknown_fields` V4 readers keep
accepting inactive status.

V5 contains every V4 field with the same meanings. See
[`capture-status-v4.md`](capture-status-v4.md).

## Maintenance

`maintenance` is required on V5 and omitted on V4. It contains no filesystem
paths, backup receipts, or private infrastructure identity:

- `enabled` and `kill_switch`: whether the owned maintenance task is configured
  and whether the operator kill-switch file is latched;
- `health` and optional `reason_code`: bounded maintenance gate, using the same
  green/yellow/red vocabulary as capture health;
- `pending_pack_manifest_count`, `packed_range_count`,
  `logical_manifest_count`, and `physical_data_object_count`: archive
  maintenance statistics;
- optional `last_scrub_at_micros`, `last_pack_index_at_micros`,
  `last_pack_data_at_micros`, and `last_retention_at_micros`: last successful
  mutating work in each category; and
- `retention_authorized`: whether a verified backup-receipt artifact currently
  authorizes packed-object retention. A configured hex token is not enough.

A green maintenance object omits `reason_code`. Yellow or red always carries
one.

## Compatibility

V5 replaces V4 only for snapshots that include maintenance. Archive, spool,
and publication formats are independently versioned. A process starting over a
V4, malformed, or foreign-build snapshot writes a fresh snapshot for its own
build and chain rather than trusting stale status state.
