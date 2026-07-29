# Canonical Block V1

`BlockEnvelope` groups a validated, deterministically ordered set of canonical
events for one chain, block height, block time, and confirmation class.

## Validation

Construction fails closed unless:

- at least one source block hash is retained;
- every event has the block's chain, height, block time, and confirmation
  class;
- every stored `EventId` equals the V1 ID derived from its canonical identity;
- event IDs are unique;
- transaction indices never regress;
- the first canonical event for each represented transaction has index zero;
- canonical event indices within one transaction are contiguous; and
- one transaction index never names multiple transaction identities.

Transaction indices may skip because a protocol transaction can emit no
canonical event. Empty committed blocks are valid and retain a deterministic
block hash.

## Canonical block hash

V1 initializes a BLAKE3 derive-key hasher with:

```text
hyperliquid-alpha-desk/canonical-block/v1
```

It hashes:

1. chain ID UTF-8 bytes with an unsigned 64-bit big-endian length prefix;
2. block height as unsigned 64-bit big-endian;
3. block time microseconds as signed 64-bit big-endian;
4. event count as unsigned 64-bit big-endian;
5. for each validated event in order:
   - transaction index as unsigned 32-bit big-endian;
   - canonical event index as unsigned 32-bit big-endian;
   - framed event ID UTF-8 bytes;
   - framed stable event-kind wire-name bytes;
   - framed canonical schema-version UTF-8 bytes;
   - market-ID count as unsigned 64-bit big-endian, then every framed market
     ID in envelope order;
   - account-address count as unsigned 64-bit big-endian, then every
     length-framed 20-byte address in envelope order;
   - the 32-byte payload hash.

The projection deliberately excludes confirmation class, source hashes,
source evidence, lifecycle timestamps, parser version, and source bytes.
Primary and independently operated committed observations of identical
canonical content must therefore produce the same canonical block hash.

Different payload, schema, market-routing, or account-routing content under one
stable event identity changes the block hash and is handled by the sequencer as
critical divergence. It is never an in-place update. Matching independently
observed events merge their sorted source evidence without changing this hash.

## Empty-block test vector

```text
chain_id: mainnet
block_height: 42
block_time_micros: 1000
event_count: 0
canonical_block_hash: ef6cc62f5122ff792f8ec41f7d2dc3eeaf6ebb323ecc671a552488918022e91a
```
