# Capture Progress Journal Contract V1

Status: frozen for the V1 read-only truth layer.

The capture progress journal is the durable handshake between the immutable
canonical archive and NATS JetStream. PostgreSQL is not the canonical event
store and it does not participate in a distributed transaction with either
system. It records enough exact identity to discover and retry every
cross-system boundary after a crash.

## Required ordering

For each committed block, the only valid order is:

1. append and verify the immutable canonical archive object;
2. record the archive-bound publication plan in PostgreSQL;
3. publish the block marker and every canonical event;
4. record each JetStream server acknowledgement;
5. advance the contiguous per-chain cursor;
6. acknowledge the source adapter cursor.

The durable cursor cannot skip the configured first block or any subsequent
height. It cannot advance until every planned publication has a matching
JetStream acknowledgement. Repeating any completed step with identical
identity is accepted; changing a block hash, archive receipt or manifest hash,
message ID, subject, payload hash, stream, sequence, duplicate flag, or
acknowledgement time is rejected.

## Durable identity

`capture_archived_blocks` binds a chain and unsigned 64-bit block height to:

- the canonical block hash;
- archive receipt ID;
- archive manifest ID and SHA-256;
- archive object SHA-256 and schema fingerprint, so recovery can reconstruct
  the exact committed block marker rather than inventing receipt fields;
- exact publication count;
- archive completion time;
- state: `archived_pending`, `publishing`, `acknowledged`, or `quarantined`.

`capture_block_publications` stores the contiguous zero-based publication
ordinal, message ID, exact subject, and payload SHA-256. Acknowledgement
columns are all-null or all-present. The database rejects zero stream
sequences, malformed identities, out-of-range unsigned values, incomplete
publication plans, and an acknowledged block with missing receipts.

`capture_sequencer_cursors` is hash-bound back to its archived block. The
cursor version starts at one and increments exactly once per contiguous block.
All block heights, stream sequences, and cursor versions are PostgreSQL
`numeric` values bounded to the complete Rust `u64` domain; the Rust adapter
uses checked decimal text conversion and never narrows them to `bigint`.

## Transactions and concurrency

Writes use serializable transactions and lock the chain-progress row.
Competing writers for one chain therefore serialize before inspecting or
changing a block, acknowledgement, or cursor. Database constraints remain
fail-closed if application validation is bypassed.

The V2 migration refuses to run when the legacy cursor table contains rows.
Those rows predate the publication journal and cannot be assigned invented
publication receipts. An operator must preserve and reconcile that evidence
before applying the migration.

## Recovery

Startup loads pending blocks in ascending height order with a caller-supplied
bound. It loads the exact plan and existing acknowledgements for each block,
verifies their archive bindings, republishes only missing publications, then
attempts contiguous cursor advancement. A missing chain origin, invalid
durable row, archive mismatch, fork, or conflicting acknowledgement prevents
readiness.

PostgreSQL connection creation is deliberately outside the storage adapter.
The long-running application owns TLS roots or client identity, password or
credential-file loading, connect/query timeouts, reconnect policy, the
connection driver task, cancellation, and task joining. The adapter never
serializes a connection string or includes database errors in public status.

## Evidence gates

- `just postgres-migration-smoke` runs both migrations on the pinned
  PostgreSQL 18.4 image, proves the full `u64` boundary, rejects overflow and
  incomplete plans, and proves the legacy-cursor preflight.
- `progress_store` exercises the backend-neutral transition semantics.
- `postgres_progress` is explicitly selected with
  `ALPHA_DESK_POSTGRES_TEST_URL` and proves persistence across a real
  disconnect/reconnect. Its default skip is not runtime evidence.
