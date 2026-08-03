# Raw source archive format v2

Status: implemented for monotonic byte-offset sources in the unreleased M2
truth-layer slice. This is an explicit sibling of the frozen
[`archive-manifest-v1`](archive-manifest-v1.md) raw format. V1 wire structs,
manifest bytes, object paths, and native-offset replay semantics are unchanged.

## Purpose and authority

Some source transports expose byte positions that are stable native evidence
but are not dense record numbers. V2 preserves those offsets and binds each
row to the contiguous `LocalRecordSequence` assigned by the verified capture
spool. Native offsets answer where bytes came from; local sequences answer the
durable physical replay order. Neither may be derived from the other.

V2 accepts only `monotonic-byte-offset` batches. Within a batch, native offsets
must be strictly increasing in one cursor epoch and may contain gaps. The local
sequence span is nonzero, inclusive, and exactly matches the Parquet row count:

```text
row_count = last_local_sequence - first_local_sequence + 1
```

Across a source catalog, local sequence spans are strictly contiguous. Native
ranges in the same epoch may not overlap. A per-hour partition may contain
local-sequence gaps because intervening records can belong to another receive
hour, but it may not contain overlaps.

## Physical isolation

V2 uses a separate dataset and global manifest namespace:

```text
chain=<encoded-chain>/dataset=raw_source_observations_byte_v2/source=<encoded-source>/
  date=YYYY-MM-DD/hour=HH/
    objects/
      epoch=<encoded-epoch>/
        sequences=<first>-<last>/
          offsets=<start>-<end>/part-<sha256>.parquet
    manifests/partition-<sha256>.json
  manifests/catalog-<sha256>.json
  CURRENT

_manifests/raw-byte-v2/manifest-<sha256>.json
```

One chain/source identity may exist under exactly one raw cursor policy. V1
and V2 writers share one source-policy advisory lock, check both dataset
visibility boundaries while holding it, and fail closed if the other policy
already has a checked regular-file `CURRENT` pointer. The shared lock retains
the frozen V1 lock location, so a V2 writer creates an otherwise inactive V1
source directory. Archive inspection and both replay APIs therefore define
policy activation only through checked `CURRENT` state: unreachable directories
and objects remain invisible, dual-active policies fail closed, and symlinked
or inaccessible policy paths are errors rather than absence.

The Parquet row schema is the frozen raw V1 schema. Local sequence is an
object-global ordinal derived from `first_local_sequence + row_index`; it is
not duplicated as a query column. The object path, V2 batch manifest, and
rolling hash bind its range.

## Explicit V2 manifests

V2 does not add optional fields to a V1 type. Its batch descriptor adds these
required fields in addition to the V1 source, cursor, spool, time, parser, and
rolling-content evidence:

| Field | Constraint |
| --- | --- |
| `cursor_policy` | exactly `monotonic-byte-offset` |
| `first_local_sequence` | nonzero first row sequence |
| `last_local_sequence` | inclusive last row sequence |

Batch references repeat both the native cursor range and local sequence range.
Partition and catalog documents repeat `cursor_policy`, use explicit V2 schema
identifiers, and reject unknown fields. Exact retries must match both ranges
and the full immutable batch descriptor. Any overlap, collision, sequence gap,
path alias, hash mismatch, generation fork, removed history, or policy change
fails closed.

Publication remains:

1. immutable Parquet object;
2. immutable content-addressed batch manifest;
3. immutable partition generation;
4. immutable source-catalog generation;
5. atomically replaced and fsynced source `CURRENT` pointer.

Objects or manifests written before a failed `CURRENT` advance are unreachable
orphans and are never replayed.

## Rolling content hash

The V2 rolling SHA-256 is domain separated with
`hyperliquid-alpha-desk/raw-rolling-content/v2`. It frames the cursor-policy
name and, for every row, its local sequence before the complete V1 raw
observation evidence: chain, source identity/version/class, native epoch and
offset, receive times, parser schema, content hash, warnings, and exact payload.
Changing a sequence assignment therefore changes the manifest even when every
native byte and payload is unchanged.

## Replay and verification

`read_observations_by_sequence` is the V2 replay API. It snapshots and verifies
the source catalog and partition chains to their roots, verifies every selected
batch/object before returning an iterator, enforces configured row and byte
limits, and yields owned observations with their exact local sequences. A
missing row or sequence gap returns `RangeUnavailable` before the first item.

The native `read_observations` API remains V1-only and requires dense native
offsets. This deliberate separation prevents a sparse byte range such as
`19, 20, 47` from being mistaken for 29 records.

CRC32C, BLAKE3, and SHA-256 provide corruption and chain evidence, not producer
authentication. An independently anchored spool/archive chain tip is still
required against an attacker who can rewrite the full local archive.
