# Capture spool format v1

Status: implemented for the unreleased Stage 1 truth-layer slice. The format is local,
append-only, and intended for deterministic recovery and replay. It is not a transport protocol
and does not by itself authenticate the producer.

## Invariants

- A caller may acknowledge observations only through an `AppendReceipt` returned after the
  corresponding bytes have completed `sync_data`.
- `FsyncEveryRecord` returns one receipt per append. `FsyncEvery` returns `None` until its bounded
  record-count threshold causes a sync; that receipt covers the latest cursor in the completed
  batch. While a batch is pending, `next_sync_deadline` exposes the absolute deadline and the
  runtime must call `flush_due` from its timer at or before that instant. An explicit `flush`
  follows the same receipt rule.
- A segment contains observations from exactly one source ID, source version, and cursor epoch.
  Cursor offsets may advance or repeat but must not regress.
- An incomplete final frame is recoverable. A malformed header, invalid length, checksum failure,
  payload-hash failure, or corruption in any complete frame fails closed without truncating the
  evidence.
- Closing a non-empty segment syncs the segment and publishes exactly one immutable manifest.
  Manifests form a BLAKE3 hash chain over their exact UTF-8 JSON bytes, including the final newline.

All integers are little-endian. Byte offsets in errors and receipts are absolute offsets from the
start of the segment.

## Segment name

`segment-<sequence as at least 10 decimal digits>.hlsp`

Sequences are unsigned 64-bit values. Directory inspection requires closed sequences to be
strictly contiguous and permits at most one complete open segment after the last closed segment.

## Header

The header is encoded once when the segment is created and synced before any append is accepted.
`header_len` is the total header size, including `magic` and `header_len`.

| Field | Encoding | Constraint |
| --- | --- | --- |
| `magic` | 8 bytes | ASCII `HLSPV001` |
| `header_len` | `u32` | 66 through 4096 bytes |
| `source_id_len` | `u16` | 1 through 256 bytes |
| `source_id` | UTF-8 bytes | trimmed, no control characters |
| `source_version_len` | `u16` | 1 through 256 bytes |
| `source_version` | UTF-8 bytes | trimmed, no control characters |
| `schema_version_len` | `u16` | 1 through 256 bytes |
| `schema_version` | UTF-8 bytes | trimmed, no control characters |
| `segment_sequence` | `u64` | matches the file and manifest sequence |
| `created_at_micros` | `i64` | non-negative Unix timestamp in microseconds |
| `producer_build_hash` | 32 bytes | raw build-identity digest |

Unknown trailing header fields are not accepted in v1. A future format revision must use a new
magic or a separately specified compatibility rule.

## Record frame

Each record begins immediately after the preceding frame. `record_len` is the number of bytes
after the length field, including `crc32c`. A v1 reader rejects declared records larger than
`256 MiB + 4096 bytes` before allocating their body.

| Field | Encoding | Constraint |
| --- | --- | --- |
| `record_len` | `u32` | bytes from `crc32c` through `content_blake3` |
| `crc32c` | `u32` | Castagnoli CRC over every following body byte |
| `cursor_epoch_len` | `u16` | 1 through 256 bytes |
| `cursor_epoch` | UTF-8 bytes | trimmed, no control characters |
| `cursor_offset` | `u64` | non-regressing within the segment |
| `observation_class` | `u8` | discriminant table below |
| `received_wall_micros` | `i64` | non-negative Unix timestamp in microseconds |
| `received_monotonic_nanos` | `u64` | producer-local monotonic time |
| `parser_schema_len` | `u16` | 1 through 256 bytes |
| `parser_schema_version` | UTF-8 bytes | trimmed, no control characters |
| `payload_len` | `u32` | 1 through 256 MiB |
| `payload` | raw bytes | source payload, preserved exactly |
| `content_blake3` | 32 bytes | BLAKE3 of `payload` only |

Observation-class discriminants are stable within v1:

| Value | Class |
| ---: | --- |
| 1 | committed block |
| 2 | auxiliary order status |
| 3 | auxiliary book diff |
| 4 | auxiliary ledger |
| 5 | snapshot |
| 6 | historical block |
| 7 | public market data |
| 8 | provisional feed |
| 9 | provisional mempool |

Zero, unknown values, unexpected trailing bytes, invalid UTF-8, and a payload hash inconsistent
with the payload are complete-record corruption.

## Recovery

`recover_open_segment` scans from the validated header:

1. A clean end-of-file is left unchanged.
2. End-of-file inside a final length prefix or declared final record truncates to the end of the
   previous complete record and syncs the repaired segment.
3. A complete frame with an invalid length, CRC, field, or payload hash returns an error and does
   not modify the file.
4. Header truncation or corruption returns an error and does not modify the file.

Recovery is only for an open segment. A segment with a published close manifest is immutable and
must be quarantined rather than repaired when verification fails.

## Closed-segment manifest

The close manifest is named `segment-<sequence>.hlsp.manifest`. Hashes are 64 lowercase
hexadecimal characters; cursors use their Serde object form with `epoch` and `offset`.

| Field | Meaning |
| --- | --- |
| `schema_version` | `hl-spool-manifest-v1` |
| `segment_sequence` | segment sequence |
| `segment_file` | exact basename of the sibling `.hlsp` file |
| `source_id` | source ID copied from the segment header |
| `source_version` | source version copied from the segment header |
| `spool_schema_version` | spool schema copied from the segment header |
| `producer_build_hash` | producer build digest copied from the segment header |
| `file_size_bytes` | final segment size |
| `record_count` | number of complete records; never zero |
| `min_cursor` | first record cursor |
| `max_cursor` | final record cursor |
| `segment_blake3` | BLAKE3 of the complete segment bytes |
| `previous_manifest_blake3` | BLAKE3 of the exact preceding manifest bytes, or `null` for genesis |
| `closed_at_micros` | non-negative close timestamp |

Publication order is:

1. sync all pending records and the segment;
2. compute segment facts from the completed file;
3. create a new `.hlsp.manifest.tmp`, write all JSON bytes, and sync it;
4. rename it to `.hlsp.manifest`;
5. sync the containing directory.

Existing final or temporary manifests cause close to fail. They are never overwritten.

## Verification and trust boundary

Run:

```sh
cargo +1.97.1 run -p spool-inspect --locked --offline -- verify <spool-path>
```

Directory verification recalculates file sizes, segment BLAKE3 hashes, record counts, cursor
bounds, header identity, and every manifest-chain link. It accepts one complete open tail for a
running capture process and rejects an incomplete tail until recovery is explicitly invoked.

CRC32C and BLAKE3 detect accidental corruption and unanchored modification; they are not digital
signatures. An attacker able to rewrite every segment and manifest can rebuild the chain.
Long-running evidence runs should therefore record the emitted chain tip in an independent,
append-only run ledger or signed evidence package.
