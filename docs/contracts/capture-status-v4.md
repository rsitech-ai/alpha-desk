# Capture Status V4

> Frozen compatibility contract. Current runtimes publish V5; see
> [`capture-status-v5.md`](capture-status-v5.md). V4 remains a supported
> read schema for inactive snapshots that omit `maintenance`.

`hl.capture.status.v4` extends the frozen V3 committed-source status with
observable Node V1 auxiliary capture state. It contains no payloads, filesystem
paths, connection strings, credentials, or private infrastructure identity.

All V3 committed-source, failover, backlog, disk, and readiness fields retain
their meanings. See [`capture-status-v3.md`](capture-status-v3.md) for that
frozen contract.

## Auxiliary sources

`auxiliary_sources` is omitted when no `node-line` adapters are enabled. When
present, it is sorted by `source_id`, contains no duplicates, and is limited to
16 entries so the complete status remains below its 16 KiB publication bound.
Each entry contains:

- `source_id`: the configured public source identity, never its path;
- `health`: `starting`, `healthy`, `quarantined`, or `latched`;
- `qualification`: `unqualified` or `qualified`; M3 Node V1 capture remains
  `unqualified` until a reviewed production qualification manifest is
  activated;
- optional `cursor_epoch` and `durable_offset`: one atomic native file cursor
  (identity plus byte offset) acknowledged after verified raw archival;
- optional `tail_cursor_epoch`: the identity of the file currently being
  tailed; during rotation this can differ from `cursor_epoch` while older-file
  records are awaiting verified group commit;
- optional `local_sequence`: the greatest contiguous process-local sequence
  acknowledged after verified raw archival;
- `spool_records`: cumulative fsynced records for the source, including
  archive-verified segments already pruned from the hot spool;
- `unarchived_records`: fsynced records in the bounded group-commit window
  which are not yet acknowledged as archived;
- optional `unread_bytes`: bytes currently visible beyond the adapter read
  offset; this is byte lag, not a record count;
- `partial_line`: whether the visible unread suffix lacks a newline and is
  therefore excluded from capture;
- optional `last_durable_wall_micros`: receive time of the last archived and
  acknowledged record; and
- optional `quarantine_reason`: the durable parser/schema disposition which
  keeps the source quarantined; and
- optional `last_error_reason`: the current retry or latched failure reason,
  kept separate from durable quarantine history; and
- `restart_reconstruction`: `not-required`, `incomplete`, or `complete`.
  Absent values deserialize as `not-required`. Recovered durable sources start
  `incomplete` until the live tail binds, then `complete`. `incomplete` and
  `complete` are invalid unless the durable cursor fields are present together.

The durable epoch, offset, local sequence, and last-durable time are present
together after the first verified group commit. `tail_cursor_epoch` may be
present by itself while a source is starting. At every snapshot,
`unarchived_records = spool_records - local_sequence`, treating an absent
local sequence as zero. A quarantined entry always carries
`quarantine_reason`; a latched entry always carries `last_error_reason`. A
quarantined source can simultaneously expose a temporary transport outage in
`last_error_reason` without losing the quarantine cause.

## Windowed throughput

`throughput_records_per_sec` and `throughput_blocks_per_sec` are unsigned
integer rates sampled over the status-heartbeat interval. They count
archive-acknowledged auxiliary records and captured committed blocks in that
window, then reset. Frozen V4-only readers may deserialize absent fields as
zero. The current dual reader treats missing last-heartbeat rates as omitted,
not zero; see [`capture-status-v5.md`](capture-status-v5.md). They describe the
last completed window only and are not a live-qualification or Stage PASS
claim.

## Durability and quarantine semantics

The Node V1 adapter may read ahead only to its configured bounded queue. Every
complete line is fsynced to the byte-offset spool. The configured provisional
durability record and delay limits bound the archive/ACK group. A group is
then sealed and verified in the V2 raw archive before its cursors are
acknowledged in order.
On crash, the spool is recovered, sealed, and archived before the source is
reopened at the recovered durable cursor.

Bounded complete-line parser failures and schema drift are not discarded.
Their exact bytes use the same local sequence and V2 archive path with a parser disposition of
`quarantine-v1:<reason-code>`. The source cursor advances only after that
quarantine evidence is verified. The source remains visibly `quarantined`
even if later records are captured successfully.

Transport and framing failures for which no bounded complete record exists
(for example an oversized or rotated partial line) fail closed without cursor
acknowledgement. The service does not claim exact quarantine bytes for an
unbounded or incomplete record.

Temporary source unavailability reports `starting` with
`source.temporary_disconnect` and retries with bounded deterministic backoff.
After the same source epoch reopens, a previously durable source returns to
`healthy` (or retains `quarantined`) and clears only the temporary error even
when no new record is available.
Cursor regression, invalid configuration, spool corruption, archive failure,
or exhausted disk reserve fail closed and become a terminal service failure.
An epoch transition is also checked against the verified V2 raw catalog, so a
pruned historical epoch cannot be re-admitted after later epochs have become
the hot checkpoint baseline.

## Compatibility

V4 replaces V3 only for the ephemeral operator snapshot. Archive, spool, and
publication formats are independently versioned and are not migrated by this
change. A process starting over a V3, malformed, or foreign-build snapshot
writes a fresh V5 snapshot for its own build and chain rather than trusting
stale status state.

Maintenance is not part of V4. Current writers emit
[`capture-status-v5.md`](capture-status-v5.md) with a required `maintenance`
object instead of adding fields under this schema id.
