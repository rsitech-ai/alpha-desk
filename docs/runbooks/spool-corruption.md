# Capture spool corruption

## Trigger and boundary

A spool parser, recovery scanner, manifest-chain, checksum, length, or
durability error is a fail-closed evidence-integrity incident. The current
synthetic node-source E2E routes every observation through the spool, verifies
its closed manifest chain, restarts from it, and requires raw Parquet parity.
That lane is useful recovery evidence, but it is not a corrupt-middle-record,
power-loss, or host-reboot qualification.

## Preserve before acting

1. Stop only the affected capture process through its normal supervisor and
   wait for bounded shutdown.
2. Record the status snapshot, stable reason code, build identity, source ID,
   expected cursor, filesystem metadata, and free space.
3. Make a read-only or copy-on-write copy of the complete affected spool root,
   including closed segments, manifests, open tail, and quarantine evidence.
4. Hash the preserved copy and store the hash with the incident record.
5. Keep the archive, PostgreSQL journal/cursor, and JetStream state unchanged;
   they are required to establish the last independently durable boundary.

## Diagnose

Run the inspector against the preserved copy, never an actively written root:

```sh
cargo +1.97.1 run -p spool-inspect --locked --offline -- \
  verify <preserved-spool-root>
```

Compare the result with the normative framing and recovery rules in
[`../formats/spool-v1.md`](../formats/spool-v1.md). A truncated final open
record may be recoverable only when the format contract explicitly permits
truncating that incomplete tail. Corruption in a closed segment, manifest,
checksum-valid record, or committed durable region is not an automatic repair.

## Recovery gate

Do not delete, rewrite, skip, or fabricate source records or advance a cursor.
Resume only after:

- the last verified source, spool, archive, publication, and cursor boundaries
  are reconciled;
- any allowed tail truncation is performed on a working copy and yields a
  fully verified spool;
- the source can replay the missing range under the approved authority
  contract; and
- a restart test proves identical raw and canonical identities, contiguous
  durable progress, and raw archive parity.

If those conditions cannot be established, keep the service non-ready and
escalate the incident with the preserved evidence.
