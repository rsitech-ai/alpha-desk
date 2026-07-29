# Event ID V1

`EventId` identifies one canonical protocol event independently of where or
when Alpha Desk observed it.

## Encoding

V1 initializes a BLAKE3 derive-key hasher with:

```text
hyperliquid-alpha-desk/event-id/v1
```

It hashes the following values in order:

1. chain ID UTF-8 bytes, prefixed by their unsigned 64-bit big-endian length;
2. block height as unsigned 64-bit big-endian;
3. transaction identity UTF-8 bytes, prefixed by their unsigned 64-bit
   big-endian length;
4. canonical event index as unsigned 32-bit big-endian;
5. the stable `EventKind` wire name as UTF-8 bytes, prefixed by its unsigned
   64-bit big-endian length;
6. canonical schema major as unsigned 64-bit big-endian.

The lowercase 32-byte digest is rendered as `evt_` followed by 64 hexadecimal
digits.

Payload bytes are deliberately excluded. Their BLAKE3 hash is retained
separately so the same event identity with different content is treated as a
critical divergence. Source identity, source cursor, confirmation source,
receive time, ingestion time, and canonicalization time are also excluded.

## Test vector

```text
chain_id: mainnet
block_height: 42
transaction_identity: tx-7
canonical_event_index: 0
event_kind: TradeMatched
canonical_schema_major: 1
event_id: evt_80387df37a3389902e817f28474bc4e48029a85ad44d5d9a670f30a8247a5ab1
```

Changing the algorithm, field order, framing, domain-separation context, event
kind wire name, or schema major is an identity compatibility change.
