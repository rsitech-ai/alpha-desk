# Hyperliquid Node Adapter

## Status and trust boundary

Stage 1 Task 3 implements two read-only source adapters:

- `NodeBlockDirectorySource` scans the node's per-height `replica_cmds` tree.
- `NodeLineFileSource` tails newline-delimited auxiliary outputs such as fills,
  order statuses, raw book diffs, and miscellaneous ledger events.

They produce byte-preserving `SourceObservation` values behind the
`hl_protocol::BlockSource` port. They do not canonicalize events, call an
exchange API, hold credentials, sign messages, place orders, or advance a
durable cursor merely because a record was read.

The library paths are implemented and focused-test proven. The `hl-capture`
binary does not yet construct these adapters or run a capture loop, so this is
not production runtime or soak evidence.

## Official source contracts

The implementation follows the current public contracts:

- [Hyperliquid node repository](https://github.com/hyperliquid-dex/node)
- [L1 data schemas](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/nodes/l1-data-schemas)
- [Historical data](https://hyperliquid.gitbook.io/hyperliquid-docs/historical-data)
- [Info endpoint](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint)

The node writes transaction blocks below
`hl/data/replica_cmds/{start_time}/{date}/{height}`. With the corresponding
flags, it writes hourly trades, fills, order statuses, raw book diffs, and
miscellaneous events. `--batch-by-block` wraps auxiliary events in
`{local_time, block_time, block_number, events}`. Low-latency operators should
also evaluate the node's `--disable-output-file-buffering` trade-off.

Node output schemas evolve independently of this repository. Any new complete
variant fails as `source.schema_drift`, retains its exact bytes for quarantine,
and requires parser review. It is never silently skipped.

## Cursor and restart semantics

Auxiliary line streams use an exclusive byte offset after the newline
delimiter. Their cursor epoch is a versioned BLAKE3 identity over:

1. the configured stream name;
2. device and inode identity where the platform exposes them;
3. the first complete record hash.

The adapter drains an old open file before accepting a replacement path.
Replacement creates a new epoch. Shrinking the same file below the speculative
read offset is `source.cursor_regression`. A partial final line remains pending;
rotation with a partial old line is malformed evidence.

Per-height transaction blocks use the block height as the cursor offset and a
versioned identity of the configured stream plus the root directory's
device/inode. The scanner:

- rejects unsafe session/date directory layouts and non-regular
  expected-height paths;
- requires an explicit initial height and probes exact numeric height paths;
- accepts duplicate heights only when their exact content hashes match;
- rejects conflicting duplicates as schema drift;
- reports a missing height when a bounded forward probe sees later data as
  `source.range_unavailable`.

Read progress and durable progress are intentionally separate. The owning
capture loop must:

1. receive an observation;
2. append it to the strict spool;
3. obtain a durability receipt;
4. acknowledge the matching adapter cursor.

On restart, only that acknowledged cursor may be supplied. Speculatively read
records are replayed. A line-file epoch change also requires the future runtime
to close the old spool segment and open a new source-identity segment before
append.

## Quarantine

Malformed complete JSON and unknown variants expose a `NodeQuarantineRecord`
containing:

- the exact payload bytes;
- the candidate source cursor;
- the BLAKE3 content hash;
- the stable reason code.

The adapter remains pinned to that record until the caller durably retains the
quarantine evidence and calls `acknowledge_quarantine_durable`. This prevents a
parser failure from becoming silent data loss.

## Configuration

The checked example config declares a primary per-height source:

```toml
[[sources]]
id = "primary-node"
class = "committed-block"
queue_capacity = 4096
max_payload_bytes = 8388608
adapter = { kind = "node-block-directory", path = "/var/lib/hyperliquid/hl/data/replica_cmds", stream_name = "replica-cmds", start_height = 1, poll_interval_millis = 25 }
```

`node-line` adapters additionally require a `stream` value:
`trades`, `fills`, `order-statuses`, `raw-book-diffs`, `misc-events`, or
`market-metadata`. The configured observation class must match the stream.
The per-height adapter requires an explicit initial `start_height`; it never
guesses a truth boundary from whichever historical file happens to sort first.
It directly probes expected height paths across session/date directories,
rather than rescanning all historical block files for every observation.
Paths must be absolute and canonical, poll intervals and payload sizes are
bounded, and unknown keys fail startup.

Regular-file reads and JSON parsing run on Tokio's blocking pool rather than
the async scheduler. Each blocking operation remains bounded by the configured
payload limit; cancellation and backpressure deadlines are checked before and
after the blocking boundary. The runtime must provision and monitor the
blocking pool for the enabled source count and observed record sizes.

## Fixture provenance

[`fixtures/source/node-v1/manifest.toml`](../../fixtures/source/node-v1/manifest.toml)
hashes every checked-in parser fixture and explicitly records:

```toml
corpus_kind = "normalized-official-documentation-examples"
production_recording = false
```

These fixtures are normalized from public schema examples. They are not
byte-exact operator node recordings. The official historical node archive is
requester-pays and requires billing authority; this implementation did not
purchase or download it.

The pinned block-batched public trade example has a conservative deterministic
mapping documented in
[`node-v1-trade-mapping.md`](../formats/node-v1-trade-mapping.md). It remains
auxiliary reconciliation evidence and cannot advance the committed watermark.

Production qualification still requires a non-secret, redistribution-reviewed
corpus captured from the exact deployed node version. The corpus must cover
every enabled output flag, block-batched and unbatched forms, real rotation,
node restart overlap, empty blocks, schema drift, and the maximum observed
record sizes.

## Focused verification

```sh
cargo +1.97.1 test -p hl-protocol --test node_golden --locked --offline
cargo +1.97.1 test -p hl-capture --test node_adapter --locked --offline
cargo +1.97.1 clippy -p hl-protocol -p hl-capture --all-targets --locked --offline -- -D warnings
```

The adapter tests cover byte preservation, partial writes, rotation, restart
from durable progress, truncation, cancellation, quarantine, per-height gaps,
and conflicting duplicate heights.
