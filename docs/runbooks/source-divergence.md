# Source Divergence Runbook

## Trigger

Use this runbook when the sequencer emits `Quarantine`, reports `Red`, or
records either reason code:

- `sequencer.conflicting_canonical_block`
- `sequencer.conflicting_source_block_hash`
- `sequencer.conflicting_event_source_evidence`

The same canonical height/content identity with different content is a
critical divergence. It is never treated as an update or normal duplicate.

## Immediate actions

1. Preserve both sources and all spool manifests. Do not overwrite, compact, or
   remove the conflicting evidence.
2. Stop committed advancement for the affected chain and suppress dependent
   signals. Keep unrelated raw capture running only while its safety checks
   remain green.
3. Record the deterministic incident ID, chain, height, source IDs, canonical
   block hashes, source block hashes, source cursors, parser/schema versions,
   and producer build IDs. Do not place raw payloads in incident metadata.
4. Verify the involved spool paths independently:

   ```sh
   cargo +1.97.1 run -p spool-inspect --locked --offline -- \
     verify <directory-or-segment>
   ```

5. Copy incident evidence to a read-only investigation location using an
   operator-approved export procedure. Hash the copied bytes before parsing.

## Reproduction

For the committed normalized public Node V1 corpus, reproduce canonical output
without overwriting an earlier result:

```sh
cargo +1.97.1 run -p canonical-inspect --locked --offline -- \
  canonicalize --root . \
  --manifest fixtures/canonical/node-v1/inspect.toml \
  --output target/divergence-repro.json
```

That command proves only the pinned normalized corpus. Until the planned
incident-export/source-divergence tool exists, do not claim arbitrary operator
recordings were reproduced by `canonical-inspect`.

Compare, in order:

1. Exact source byte hashes and spool manifest chains.
2. Parser/schema and producer versions.
3. Source-specific block hashes.
4. Canonical event IDs and payload hashes.
5. Canonical block hashes and event counts.

If canonical hashes match but the same source ID reports a different raw block
hash, investigate source mutation, cursor reuse, or a capture boundary defect.
If canonical hashes differ, retain both canonical projections and reproduce the
first differing event.

## Resolution

- A deterministic parser defect requires a versioned fix, historical replay,
  and correction/rebuild procedure; never edit archived raw evidence.
- A source defect requires independent verification and explicit source
  requalification before it can re-enter the committed lane.
- Operator resolution requires a recorded reason, named approval, and a clean
  restart/replay showing identical final watermarks and hashes.
- The sequencer's red latch is intentionally not clearable in process. Restart
  only from reviewed durable state after evidence is preserved.

## Current implementation boundary

The deterministic incident ID, quarantine record, bounded red-latched
sequencer, and spool/canonical inspection tools exist. PostgreSQL integration,
immutable archive receipts, automated incident export, runtime restart, and
operator approval APIs are not implemented yet.
