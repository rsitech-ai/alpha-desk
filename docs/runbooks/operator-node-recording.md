# Operator Node Recording Runbook

Status: preparation only. Running a live Hyperliquid node and retaining an
operator corpus require explicit operator authority and sufficient local
resources. Do not place private recordings in Git.

## Purpose

Capture simultaneous, byte-exact committed `replica_cmds` blocks and
block-batched trade output from one identified node process/build. The result
is evidence for designing and testing the committed parser and join; it is not
automatically a qualified source.

## Before capture

1. Use an operator-owned Ubuntu 24.04 environment that meets the current
   official node requirements. Record OS/kernel, filesystem, clock source, disk
   headroom, and process supervision.
2. Retain the exact node repository URL/commit, downloaded or built binary,
   binary SHA-256, build command and material hash, and official signature
   fingerprint/verification output. Never log credentials or private keys.
3. Choose a new stable `node_instance_id` for the physical process and distinct
   source IDs for committed blocks and trade batches.
4. Create a private, access-controlled recording root outside the repository.
   Record its ownership, retention policy, and redistribution classification.
5. Preflight enough disk for raw outputs, immutable copies, hashes, spool/archive
   evidence, and restart overlap. Stop before capture if the bound cannot be
   maintained.

## Required node output contract

Retain the complete argv. It must include exactly:

```text
--write-trades
--batch-by-block
--replica-cmds-style actions-and-responses
```

Do not add `--write-fills`: the node documents that it overrides
`--write-trades`, which would invalidate this recording contract.

Record the buffering choice explicitly. For evidence runs, disabled buffering
uses `--disable-output-file-buffering`; the manifest must agree with argv.
Do not infer the process command from a shell-history fragment or editable
configuration file—capture the supervised process identity and exact argv.

## Raw-first capture

1. Start both readers before the selected evidence range.
2. Copy only complete source records. Preserve original bytes, source path,
   inode/file identity, rotation epoch, byte offset, receive wall/monotonic
   times, parser version, and BLAKE3 content hash.
3. Fsync each independent spool/raw archive before acknowledging its adapter.
4. Treat partial final lines as pending. Treat malformed complete lines,
   rotation ambiguity, cursor regression, or divergent duplicates as durable
   quarantine.
5. Never join by timestamp proximity and never relabel an auxiliary trade row
   as committed during capture.
6. Retain deliberate clean restart and abrupt process-restart overlap. Record
   file rotations, duplicate ranges, gaps, downstream outage, and shutdown.

## Minimum corpus coverage

The byte-first review must find and retain evidence for:

- action-bearing blocks with trades;
- explicit no-trade/empty-block behavior;
- multiple trade-producing transactions with non-trade transactions between;
- multiple matches within one transaction;
- repeated-looking legitimate rows at distinct match ordinals;
- buyer/seller `start_pos`, order IDs, optional CLOID/TWAP IDs, price, quantity,
  market, and transaction hash relationships;
- node restart overlap and output-file rotation;
- exact duplicate, conflicting duplicate, missing/gap, malformed record, and
  schema-drift cases;
- market-catalog changes and the largest observed valid record.

If the committed response does not prove the complete ordered match projection,
or the source does not provide deterministic evidence that a trade batch is
complete/empty, the affected trade/block remains evidence-only.

## Manifest and verification

1. Close the selected range without editing source files.
2. Hash every retained file with SHA-256 and record its nonzero size, role,
   rotation sequence, and first/last native cursor evidence.
3. Build the raw compact canonical V1 JSON using the complete ordered template
   and bounds in `docs/contracts/node-source-qualification-v1.md`. M1 has no
   public manifest builder; the strict decoder plus read-only getters are the
   generation/review boundary.
4. Compute the manifest SHA-256 from the exact bytes. Decode it with the local
   contract and verify byte-for-byte canonical re-encoding.
5. Independently review build/signature evidence, every file hash, source
   correlation, privacy, license, and redistribution class.
6. Keep the registry unchanged until the committed projection, empty-batch
   behavior, restart replay, and reviewer decision are all complete. Adding a
   digest to the registry is a separate reviewed code change.

## Evidence boundary

- Canonical decode with an empty registry: `repo-ready` contract evidence only.
- Same-build private corpus plus verified parser/join/restart replay:
  candidate `runtime-proven` evidence for this one source profile.
- It does not prove independent-source reconciliation, Stage 1, Stage 2,
  deployment, public release, or trading safety.
