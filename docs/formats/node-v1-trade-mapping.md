# Node V1 Trade Mapping

This document freezes the first executable public Node V1 source-to-canonical
mapping. It applies only to complete `--write-trades` records in the documented
`--batch-by-block` or `--stream-with-block-info` envelope:

```text
{local_time, block_time, block_number, events}
```

The checked fixture is a normalized combination of the public trade example
and public block-wrapper contract. It is hashed in
`fixtures/source/node-v1/manifest.toml` and is not a production node recording.

## Admission and disposition

- A standalone trade without block metadata is retained as evidence only.
- An empty block batch has the explicit `EmptyBlock` disposition.
- A complete block-bearing trade batch maps in source array order.
- Known records without implemented canonical semantics have the explicit
  `UnsupportedCanonicalSemantics` evidence-only disposition.
- Unknown markets, invalid transaction hashes, invalid addresses, non-positive
  or unrepresentable fixed-point values, invalid block times, and
  non-contiguous repeated transaction hashes fail closed with stable mapping
  reason codes. Order-status and L4 batches apply the same fail-closed rule to
  synthetic transaction ids: a later reappearance of an earlier oid is
  `NonContiguousTransaction`.

The `trades` stream remains `AuxiliaryLedger`. Even with block metadata, mapped
events use `ProvisionalSource`; this auxiliary source cannot advance the
committed watermark.

## Field mapping

| Source | Canonical |
| --- | --- |
| wrapper `block_number` | `block_height` |
| wrapper `block_time` | `block_time`, interpreting the node's timezone-less documented form as UTC |
| event `hash` | `transaction_id` |
| first appearance of each contiguous hash | `transaction_index` |
| position within one contiguous hash | `event_index` |
| event array position | `source_evidence.source_event_index` |
| catalog lookup of `coin` | `market_id` |
| `side_info[0].user` | buyer account address |
| `side_info[1].user` | seller account address |
| `px` | checked positive `Price` |
| `sz` | checked positive `Quantity` |

`side_info[0]` is the buyer and `side_info[1]` is the seller. Maker and taker
are derived, fail-closed, from the documented aggressor fields:

- `side` `"B"` means the buyer is the taker (aggressor buy). The seller oid is
  the maker.
- `side` `"A"` means the seller is the taker. The buyer oid is the maker.
- `trade_dir_override` must be `"Na"`. Any other override is `SchemaDrift`.
- Any other `side` is `SchemaDrift`.

`maker_order_id` and `taker_order_id` are those oids. `deterministic_seed` is
reserved and is zero. Match keys for committed reconciliation are
`node-trade:{trade_id}`.

Standalone order-status and raw-book-diff records without `block_number` stay
evidence-only. Block-batched order statuses map to the existing V1 kinds
(`OrderAccepted`, `OrderCancelled`, `OrderRejected`, `OrderFilled`,
`TriggerOrderActivated`) with `ProvisionalSource` confirmation. L4 `new` and
`update` map to `OrderRested`; `remove` maps to `OrderCancelled` with remaining
quantity zero. Snapshots cannot mint a committed watermark. Schema version on
envelopes remains `"1.0.0"`.

Order-status and L4 batches use a synthetic transaction id (`node-order:{oid}` /
`node-l4:{oid}`). Grouping copies the trade path: first appearance of each
contiguous oid is `transaction_index`, position within that run is
`canonical_event_index` (reset to zero when the oid changes), and a later
reappearance of an earlier oid is `NonContiguousTransaction`. Same-oid diffs in
one run get distinct event ids from the within-group index. Distinct oids get
distinct transaction identities and restart the event index at zero, which is
what `BlockEnvelope::try_new` requires.

| Source | Canonical |
| --- | --- |
| filled `limitPx` | `OrderFilled.fill_price` (not a print; the trades stream holds the match) |
| filled `origSz` | `OrderFilled.fill_quantity` (`sz` is remaining) |
| triggered `triggerPx` | `TriggerOrderActivated.trigger_price` and `oracle_price` |
| first appearance of each contiguous oid | `transaction_index` |
| position within one contiguous oid | `canonical_event_index` |
| event array position | `source_evidence.source_event_index` |

The parser version records both mapper and market-catalog versions:

```text
node-v1-mapper-1/catalog:<catalog-version>
```

## Derived trade identity

`TradeId` uses a BLAKE3 derive-key hasher with:

```text
hyperliquid-alpha-desk/trade-id/node-v1
```

It hashes, in order:

1. chain ID as unsigned-64-bit-length-prefixed UTF-8;
2. block height as unsigned 64-bit big-endian;
3. transaction ID as unsigned-64-bit-length-prefixed UTF-8;
4. canonical event index as unsigned 32-bit big-endian.

The digest is lowercase hexadecimal prefixed with `trd_`. For the committed
fixture, the first identity is:

```text
trd_9d76b6581c97fe76b0d8e8e1bec50b7fc85ead4f7235abff2a03f9991c0e70ff
```

Changing the domain, framing, field order, or transaction-relative index is an
identity compatibility change.

## Qualification boundary

This mapping proves deterministic behavior against normalized public
documentation examples. Production qualification still requires
redistribution-reviewed recordings from the exact deployed node version,
including multi-match transactions, HIP-3 markets, block-streaming modes,
restart overlap, and schema-drift cases.
