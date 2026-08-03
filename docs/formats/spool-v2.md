# Capture spool format v2

Status: implemented for byte-offset source retention in the unreleased M2
truth-layer slice. This format extends the frozen
[`spool-v1`](spool-v1.md) segment framing without changing V1 bytes or V1
manifest decoding.

## Purpose

Some source cursors are native byte offsets. They must increase within one
file epoch, but they are not necessarily adjacent integers. V2 keeps that
native cursor intact and assigns every durably appended physical record a
separate, process-local `LocalRecordSequence`.

The local sequence is:

- nonzero;
- contiguous in physical append order across segment rotation and cursor
  epoch changes;
- unchanged by exact duplicate retries;
- reconstructed from authenticated closed-segment sequence spans on restart;
- not a source cursor and never derived from a native byte offset.

## Segment header

A byte-offset segment uses the `HLSPV002` magic. Its header has the V1 fields
in the same order, with one cursor-policy byte after `schema_version` and
before `segment_sequence`:

| Value | Meaning |
| ---: | --- |
| `1` | monotonically increasing native byte offsets |

Unknown values and trailing bytes fail closed. An `HLSPV001` header always has
the legacy contiguous-native-offset policy. An `HLSPV002` header binds the
byte-offset policy into the segment bytes; a V2 manifest whose policy does not
match its segment header is invalid.

Record framing and payload preservation are unchanged from V1. Within one V2
segment, native offsets must be strictly increasing and the cursor epoch must
be constant. An epoch change closes the current segment before the first
record of the new epoch is appended.

Byte-offset capture requires `FsyncEveryRecord`. The returned append receipt
therefore proves durability before the local sequence may be acknowledged.

## Closed-segment manifest

A V2 segment publishes an explicit `hl-spool-manifest-v2` document. V1
documents remain frozen and reject V2 fields. The V2 document contains these
additional fields before the common segment evidence:

| Field | Constraint |
| --- | --- |
| `cursor_policy` | exactly `monotonic-byte-offset` |
| `first_local_sequence` | nonzero first physical sequence in the segment |
| `last_local_sequence` | exact final physical sequence in the segment |

The required span invariant is:

```text
last_local_sequence = first_local_sequence + record_count - 1
```

The addition is checked for overflow. A missing, zero, truncated, extended,
or otherwise inconsistent span invalidates the manifest. The remaining fields
and atomic publication order are the same as the V1 close manifest.

The BLAKE3 manifest chain hashes the exact serialized V1 or V2 predecessor
bytes, including the final newline. A chain can therefore transition only
through explicit immutable documents; policy or sequence changes alter the
chain tip.

During the unreleased M2 development migration, some `HLSPV002` segments were
closed with a V1 manifest before the V2 manifest existed. The V1 document did
not declare a cursor policy or local span. Verification therefore treats a V1
manifest's policy as unbound, uses the already-hashed segment header as the
policy authority, and reconstructs physical sequence from verified record
order starting at one. The next close emits V2 and must continue at the exact
derived sequence. V1 documents are not extended with optional fields.

## Restart and verification

Directory inspection dispatches on `schema_version`, decodes into an exact V1
or V2 wire type with unknown fields denied, and verifies:

1. segment file name, sequence, byte length, and BLAKE3;
2. header source identity, build identity, spool schema, and cursor policy;
3. record count and native cursor bounds;
4. V2 local-sequence span arithmetic;
5. the exact predecessor-manifest hash chain.

`inspect_spool` performs the directory-wide predecessor-chain and sequence
continuity checks above. `CloseReceipt::load` verifies only the selected
manifest and sibling segment; it does not load predecessor manifests. A loaded
receipt therefore requires an independently verified or anchored chain context
before its predecessor integrity can be claimed. Archive code must consume the
policy and local-sequence span from the receipt and its verified chain context.
It must not accept a caller-provided first sequence or infer sequence from byte
offsets.

This unreleased M2 revision intentionally changes `CloseReceipt::manifest()`
to return the versioned `ClosedSegmentManifest` view. Callers that recover a
byte-policy writer directly can close it with the public
`close_with_local_sequence_span` method; the legacy `close` method remains the
V1-only path. No released crate version exposed the superseded API.

CRC32C and BLAKE3 remain integrity evidence, not producer authentication. As
with V1, an independently anchored chain tip is required for evidence against
an attacker who can rewrite the entire spool directory.
