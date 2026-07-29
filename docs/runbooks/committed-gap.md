# Committed Gap Runbook

## Trigger

Use this runbook when the canonical sequencer emits `RequestGap`, reports
`RedGap`, or a downstream component reports a committed watermark that does
not advance while later blocks are present.

A committed gap is a data-integrity incident. The affected committed watermark
and dependent signals stay stopped until the exact range is recovered and
verified.

## Immediate actions

1. Keep capture and spooling running if disk, checksum, and source-health
   checks remain safe. Do not discard later observations.
2. Record the chain, missing inclusive range, last committed block, each source
   cursor, build ID, schema version, spool manifest hash, and deterministic
   `inc_...` gap incident ID.
3. Suppress affected committed signals and show the incomplete watermark to
   operators. Provisional displays may continue only with their provisional
   origin visible.
4. Verify every involved spool before requesting recovery:

   ```sh
   cargo +1.97.1 run -p spool-inspect --locked --offline -- \
     verify <directory-or-segment>
   ```

5. Request the exact missing range from an independently operated complete
   committed source. When that source is configured, `hl-capture` continuously
   spools it and can perform the V1 one-way failover described below. If exact
   independent evidence is unavailable, keep the canonical cursor stopped.

## Recovery order

1. Independent complete committed source.
2. Qualified historical source retaining complete block evidence.
3. Public REST or snapshots only for reconciliation. They must never be
   promoted into missing committed event history.

Feed recovered blocks through the same parser, canonical mapper, admission
policy, and sequencer used for live capture. A recovered height may advance the
watermark only when its complete block validates and all prior heights are
contiguous.

## Verification and closure

- Compare canonical block hashes, event counts, and source evidence for the
  recovered range.
- Confirm the sequencer emits ordered `Commit` decisions through the buffered
  high height and clears `outstanding_gap`.
- After the archive milestone exists, require an archive receipt for every new
  committed block before updating `capture_sequencer_cursors`.
- Restart from the last durable cursor and replay the affected range. The final
  watermark and committed block hashes must match the uninterrupted result.
- Resolve the incident only with a recorded reason and named operator
  approval.

## Forbidden actions

- Do not skip a height, move the cursor manually, or use ingestion time to
  synthesize order.
- Do not rebuild missing committed history from snapshots, public recent
  history, or provisional feeds.
- Do not delete or rewrite source spool bytes.
- Do not re-enable dependent signals merely because later heights are
  arriving.

## Current implementation boundary

The production entrypoint supports exactly one locally verified committed
node-directory source and at most one independent committed node-directory
source. Both are fsynced and raw-archived separately. A visible primary gap:

1. parks only primary acquisition at the exact missing height;
2. requires that height to be already durable in the independent spool;
3. writes the private, checksummed, create-once
   `hl.capture.failover.v1` decision before canonical commit;
4. drains only the independent spool from that height onward; and
5. restores the independent selection on restart, with no automatic failback.

Status V3 remains yellow while the independent source is active. A missing
exact independent height or a gap in the active independent source is red and
non-ready. Repairing the primary does not clear the failover decision.

Automated historical fetching, overlap reconciliation, and automatic failback
are not implemented. Do not delete or edit the failover state to simulate
either operation. Action-bearing committed mapping also remains fail-closed
until a redistribution-approved operator corpus freezes the real node response
contract.
