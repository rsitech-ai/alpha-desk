# Immutable Archive and Manifest V1

## Scope

The V1 local archive is an operator-controlled, read-only reconstruction
source. Canonical rows preserve the exact encoded canonical-event Protobuf
envelope. Raw rows preserve the exact source payload plus source version,
observation class, cursor, receive timestamps, parser version, BLAKE3 content
hash, and deterministic warning JSON.

The archive is authoritative only through a verified manifest chain. A
Parquet file that is not reachable from a validated `CURRENT` catalog is an
orphan and is never read.

## Layout

Canonical objects are chain and hour scoped:

```text
chain=<encoded-chain>/dataset=canonical_events/
  date=YYYY-MM-DD/hour=HH/
    objects/block_start=<height>/block_end=<height>/part-<sha256>.parquet
    manifests/partition-<sha256>.json
    CURRENT
  manifests/catalog-<sha256>.json
  CURRENT
```

Raw source objects are chain, source, and receive-hour scoped:

```text
chain=<encoded-chain>/dataset=raw_source_observations/source=<encoded-source>/
  date=YYYY-MM-DD/hour=HH/
    objects/epoch=<encoded-epoch>/offsets=<start>-<end>/part-<sha256>.parquet
    manifests/partition-<sha256>.json
  manifests/catalog-<sha256>.json
  CURRENT
```

Sources whose native cursor is a sparse byte offset use the physically
isolated [raw archive V2](raw-archive-v2.md). V1 remains the dense
native-offset contract and does not accept V2 fields.

Content-addressed block-bundle and raw-batch manifests live under
`_manifests/blocks/` and `_manifests/raw/`. Components use a canonical
percent encoding. Readers reject non-canonical encodings, absolute paths,
traversal components, aliases, and symlinks.

## Publication and visibility

A writer holds both an in-process mutex and a non-blocking OS advisory lock.
It writes Parquet to a temporary file in the destination filesystem, closes
the writer, fsyncs the file, hashes it, publishes it without clobbering an
existing path, and fsyncs the directory. It then publishes immutable
content-addressed manifests. The small mutable `CURRENT` pointer is written
and fsynced last.

Canonical visibility is the dataset catalog `CURRENT`, not the partition
pointer. Advancing a canonical partition pointer before the catalog is safe:
readers continue from the previous catalog until the new catalog becomes
visible. Raw source catalogs directly reference immutable per-hour partition
heads; the source catalog `CURRENT` is their only visibility boundary.
Returning an archive receipt means the referenced object and manifest have
already passed a full decode and content-hash verification.

## Verification

Readers snapshot `CURRENT` and validate every catalog and partition
generation back to its root. Append generations retain every prior block
reference and add exactly one. Compaction generations retain the identical
ordered block-height and canonical-block-hash sequence while replacing object
references. Catalog generations change or add exactly one partition and must
descend from the prior partition head.

Raw batches cannot cross an hour boundary. Each raw partition generation adds
exactly one non-overlapping cursor range, embeds its complete source
watermark, and descends from the prior partition head. The source catalog
repeats the ordered batch references and is rejected if they differ from the
union of its partition manifests.

Before returning an iterator, a range read verifies:

- exact manifest path-to-hash binding and append/compaction chain rules;
- object path, regular-file identity, byte size, SHA-256, schema fingerprint,
  and Parquet schema;
- contiguous block or cursor coverage and configured record/byte limits;
- query columns against authoritative Protobuf envelopes or source payloads;
- canonical block hashes, source evidence, raw BLAKE3 content hashes, row
  counts, and framed rolling SHA-256 hashes.

Any defect fails before the first block or observation is yielded.

## Compaction

Compaction accepts at least two contiguous canonical blocks from one hour
partition. It verifies all inputs, writes one new immutable bundle in stable
block/transaction/event order, compares row count, block descriptors,
canonical hashes, rolling content hash, and a complete replay, then advances
the partition and catalog generations. Repeating the same request returns the
existing generation. Prior manifests and objects remain present; V1 has no
deletion policy.

## Operator inspection

```sh
cargo +1.97.1 run -p archive-inspect --locked --offline -- verify <archive-root>
cargo +1.97.1 run -p archive-inspect --locked --offline -- count <archive-root>
```

`verify` walks only reachable catalogs and fully verifies canonical and raw
objects. `count` first performs the same verification, then independently
opens each reachable canonical Parquet object through pinned DataFusion and
compares the physical row count with the manifest total. Both commands emit
one bounded summary line and return nonzero with a stable reason code on
failure. An empty directory is not a valid operator archive and fails with
`archive_inspect.empty_archive`.

The committed synthetic fixture can be checked with:

```sh
just archive-verify
just archive-count
```

## Current limitations

V1 provides a local filesystem backend. It does not implement remote object
replication, retention deletion, disaster-recovery copying, JetStream
publication, sequencer cursor advancement, ClickHouse loading, or a
long-running service. Those capabilities must consume a verified archive
receipt; they must not weaken this format boundary.
