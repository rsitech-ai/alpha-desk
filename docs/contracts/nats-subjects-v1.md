# NATS Subjects and Canonical Publication Contract V1

Status: frozen for the V1 read-only truth layer.

NATS JetStream is an operational replay and fan-out layer. It is not the
authoritative ledger. A committed message is eligible for publication only
after the immutable Parquet archive returns an `ArchiveReceipt` matching the
canonical chain, block height, and block hash.

## Exact subjects

| Subject | Stream | Producer |
| --- | --- | --- |
| `hl.v1.block.committed` | `HL_CANONICAL` | `hl-capture` |
| `hl.v1.block.provisional` | `HL_CANONICAL` | approved provisional capture |
| `hl.v1.event.fill` | `HL_CANONICAL` | `hl-capture` |
| `hl.v1.event.order` | `HL_CANONICAL` | `hl-capture` |
| `hl.v1.event.ledger` | `HL_CANONICAL` | `hl-capture` |
| `hl.v1.event.market_meta` | `HL_CANONICAL` | `hl-capture` |
| `hl.v1.event.oracle` | `HL_CANONICAL` | `hl-capture` |
| `hl.v1.state.account_delta` | `HL_STATE` | `hl-core` |
| `hl.v1.state.book_delta` | `HL_STATE` | `hl-core` |
| `hl.v1.feature.wallet` | `HL_FEATURE` | intelligence services |
| `hl.v1.feature.entity` | `HL_FEATURE` | intelligence services |
| `hl.v1.feature.market` | `HL_FEATURE` | intelligence services |
| `hl.v1.signal.candidate` | `HL_SIGNAL` | signal service |
| `hl.v1.signal.live` | `HL_SIGNAL` | signal service |
| `hl.v1.signal.resolved` | `HL_SIGNAL` | signal service |
| `hl.v1.health.data` | `HL_HEALTH` | assessed service |
| `hl.v1.health.model` | `HL_HEALTH` | model service |

Poison-message records use `hl.v1.deadletter.<original-subject>` in
`HL_DEADLETTER`. A dead-letter record must contain the original subject,
stream and consumer sequence, non-secret headers, payload SHA-256, consumer,
retry count, stable error code, and failure time. It must not contain
credentials or an unbounded error/debug string.

Capture credentials may publish only `hl.v1.block.*`, `hl.v1.event.*`, and
`hl.v1.health.data`. They cannot publish state, feature, signal, or model-health
subjects. Read-only API credentials cannot publish any subject.

The development stack enforces three separate identities:

- bootstrap may call JetStream administration API subjects and receive only
  its request replies;
- capture may publish the three capture-owned subject patterns above and
  subscribe only to request reply inboxes;
- reader may inspect streams, fetch consumer messages, acknowledge deliveries,
  subscribe to V1 data, and receive request replies, but may not publish any
  `hl.v1.*` product subject.

Anonymous access is disabled as soon as these users are configured. Local
passwords are generated under ignored `state/dev`, stored with owner-only
permissions, and injected through the process environment. The capture
publisher reads its password from an absolute, normalized, regular,
non-symlink file with no group/other permissions. Inline URL credentials,
relative paths, broad file modes, and invalid usernames fail closed. Production
may use the same path boundary with NATS credentials files or an externally
issued user/password secret; neither secret form belongs in Git or status.

`just dev-up` runs a live permission probe after stream bootstrap. It proves
the bootstrap and reader inspection paths, a capture-owned publication, denial
of capture stream administration/state publication, denial of reader
publication, and denial of anonymous access.

## Event routing

- `fill`: partial/full order fills, TWAP slice fills, matched trades, and
  liquidation/backstop fills.
- `order`: accepted, rested, modified, cancelled, rejected, trigger activation,
  and TWAP start/completion lifecycle events.
- `oracle`: oracle and funding-rate updates.
- `market_meta`: market halt/resume, open-interest cap, margin table, market,
  asset-context, DEX, and outcome metadata/lifecycle events.
- `ledger`: every remaining committed canonical event, including transfers,
  fees, funding cash flows, account/margin/leverage changes, liquidation
  starts, and position settlement.

The Rust mapper matches every closed `EventKind` variant explicitly. Adding an
event kind without choosing a subject is a compile error.

## Message identity and bytes

Canonical event payloads are the exact encoded
`hl.canonical.v1.CanonicalEventEnvelope` bytes. Their `Nats-Msg-Id` is the
canonical lowercase `EventId` string. Reusing an event ID with another
publication payload hash, block hash, archive manifest hash, or subject is a
critical divergence and is rejected before transport.

`hl.v1.block.committed` is a deterministic V1 boundary marker. Its
`Nats-Msg-Id` is `blk_<lowercase canonical block hash>`. The binary marker
binds:

- schema identifier, chain, height, block time, and confirmation class;
- canonical block hash;
- archive receipt ID, manifest ID/hash, and archive schema fingerprint;
- ordered source IDs and source block hashes;
- ordered event IDs, kinds, payload hashes, and SHA-256 hashes of the exact
  canonical event envelopes.

The complete canonical event envelopes are published separately on their
event subjects. Consumers checkpoint a block only after the committed block
marker and all events named by it have been applied atomically.

Every message supplies:

- `Nats-Msg-Id`;
- `Nats-Expected-Stream`;
- V1 schema identifier;
- chain and block height;
- canonical block hash;
- archive receipt ID and manifest SHA-256;
- publication payload SHA-256.

The publisher awaits the JetStream server acknowledgement. A send/enqueue
success without the acknowledgement is not a durable publication receipt.

## Stream policy

All six streams use file storage and limits retention. Development uses one
replica and production uses three. Initial maximum age is six hours and may be
raised only to a measured maximum of twenty-four hours. Each stream has
explicit byte/message limits, `discard old`, and a duplicate window covering
the supported publication-retry interval.

The duplicate window is operational protection, not durable identity storage.
JetStream compares `Nats-Msg-Id`, not payload bytes, and forgets IDs outside
the window. The capture progress journal and application-side hash binding are
therefore required even when server deduplication is enabled.

Development acknowledgements from a one-replica stream do not prove
host-power-loss durability. Production qualification requires a three-member
JetStream cluster, quorum acknowledgement, restart tests, and retained Parquet
archive recovery.

## Consumer policy

Production consumers are durable pull consumers with explicit per-message
acknowledgements, bounded `max_ack_pending`, bounded delivery attempts, and a
reviewed backoff. Effects are idempotent by event ID. The consumer persists its
last completed block checkpoint only after every effect for that block commits.
Acknowledgement loss and redelivery must not duplicate state.
