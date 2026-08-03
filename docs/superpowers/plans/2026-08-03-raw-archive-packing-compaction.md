# Bounded Raw Archive Packing and Compaction

## Goal

- User-visible outcome: `hl-capture run` can retain continuous auxiliary node
  streams without creating an unbounded small-file or catalog-rewrite storm,
  while every archive-before-ACK receipt and recovery checkpoint remains
  independently verifiable.
- How to see it working: a retained evidence run reports 128–512 MiB packed
  Parquet objects where enough data exists, at most one explicitly underfilled
  sealed tail per source/time partition, bounded index depth and active-tail
  entries, exact replay before and after compaction, successful crash recovery
  at every publication boundary, and stable object/index growth during a
  24-hour soak.
- Completion boundary: focused tests and short synthetic E2E establish
  `repo-ready`; a retained 24-hour process soak with compaction, analytical
  reads, restart, scrub, and restore establishes `runtime-proven` for this
  storage path only.
- Authority boundary: local code, tests, documentation, and synthetic local
  evidence may change. Do not delete retained user archives, migrate an
  operator archive, start external infrastructure, push, publish, or deploy
  without separate authority.

## Current State

- Relevant paths:
  - `crates/canonical-archive/src/raw_v2.rs`
  - `crates/canonical-archive/src/raw_policy.rs`
  - `crates/canonical-archive/src/inspection.rs`
  - `crates/storage-ports/src/archive.rs`
  - `services/hl-capture/src/raw_archive.rs`
  - `services/hl-capture/src/auxiliary_checkpoint.rs`
  - `services/hl-analytics/`
  - `tools/archive-inspect/`
  - `tools/ci/capture-e2e.sh`
- Existing behavior at M3A commit
  `0d8cfaa0eefac982e151cac616c4dd0813101d5f`:
  - Every bounded auxiliary group commit is fsynced to a source spool, sealed,
    written as a V2 Parquet object, verified, checkpointed, and only then ACKed.
  - Each group creates an immutable batch manifest, partition generation,
    catalog generation, and Parquet object. Partition and catalog generations
    copy all prior batch references and verification walks the full append
    chain.
  - The checkpoint binds the exact V2 manifest IDs, spool hashes, archive path
    identity, sequence/cursor, and quarantine history. Hot spool pruning relies
    on those receipts remaining verifiable.
  - Canonical block compaction is idempotent and immutable, but byte-offset raw
    observations have no packing or compaction path.
- Constraints:
  - Archive-before-ACK provenance cannot be weakened to spool-only ACK.
  - Frozen V1/V2 formats and retained receipts remain readable.
  - Native byte offsets may be sparse, epochs may rotate, and local sequence is
    the only exact cross-epoch contiguous ordering key.
  - A pack may not cross chain or source. It may contain multiple epochs,
    parser dispositions, source versions, spool segments, and logical commits
    only when each original descriptor remains independently authenticated.
  - Raw bytes, warnings, receive clocks, hashes, and logical ordering must
    replay byte-identically.

## Target State

- New byte-offset archives use a versioned V3 dataset. Existing V2 archives
  remain readable and writable in compatibility mode until an explicit,
  non-destructive maintenance import produces and verifies a V3 root. Capture
  never silently rewrites or deletes a V2 archive.
- V3 stores immutable logical commit manifests for archive-before-ACK receipts,
  one authoritative content-addressed copy-on-write sequence tree, and a
  rebuildable receipt-lookup hint keyed by original manifest SHA-256. Sequence
  nodes and sorted hint locators are stored as checksummed pages inside
  immutable bounded index packfiles; refs address exact
  `(pack hash, offset, length, page hash)` slices. One bounded append-only index
  journal generation covers new logical commits until index packing. Append
  and fsync record bytes first; each root ref then binds the exact journal file
  identity/generation, committed prefix length, record count, and
  domain-separated prefix rolling/Merkle hash. A committed prefix is never
  truncated or overwritten, so a leased older root remains verifiable while
  later roots bind longer prefixes. Fixed
  fanout/page/pack/journal limits bound node size and active small files;
  append and range lookup perform logarithmic authoritative work. One
  root-bundle manifest binds only the authoritative sequence root, exact
  journal-generation prefix ref, logical head sequence, generation, and prior
  root hash. The
  hint has a separate rebuildable pointer and can never authenticate data.
- A leaf entry is either an uncompacted logical commit or a packed range. A
  pack manifest embeds the canonical bytes and hashes of every superseded
  logical manifest, exact row-slice boundaries, original object hashes,
  ordered sequence/epoch/cursor summaries, combined rolling digest, and one
  verified Parquet object descriptor.
- Compaction selects exact consecutive logical entries within one source and
  safe time partition, writes and verifies the output object, publishes the
  immutable pack manifest, then atomically publishes a new verified index root.
  Repeating the same selection is idempotent.
- V3 receipts and auxiliary checkpoint V2 entries bind both original manifest
  ID and exact local-sequence range. `verify_raw_manifest_at_sequence(id,
  range)` looks up that authoritative sequence range and authenticates the
  original immutable object or its exact embedded slice in a reachable pack.
  The optional receipt hint can locate legacy ID-only requests, but every hint
  result is reauthenticated against the authoritative sequence tree. A
  missing/corrupt hint fails closed with `archive.receipt_index_rebuild_required`
  or is rebuilt from a leased authoritative root; it can never prove a false
  receipt. Checkpoints therefore survive physical packing and later garbage
  collection without changing original manifest IDs.
- Every reader first loads root `R`, opens the regular non-symlink
  root-specific lease file, acquires an OS-owned shared lock, rereads and
  verifies `R`, and retains the open lease guard for the full iterator,
  DataFusion query, inspection, scrub, or export lifetime. Process exit releases
  the lock without trusting a stale PID or wall-clock expiry. Reader work is
  bounded/cancellable; a stuck live reader blocks GC and turns maintenance
  unhealthy rather than allowing unsafe expiry. GC takes an exclusive lock for
  every root it intends to reclaim and marks CURRENT, all shared-locked roots,
  configured retained roots, verified backup roots, recovery-checkpoint roots,
  import roots, and the pre-cutover V2 fallback before planning any unlink.
  Immediately before each unlink it rechecks CURRENT, the complete mark set,
  exclusive leases, plan digest, path/hash, backup receipt, and deletion
  journal under the writer lock. Unlocked stale lease files are removed only
  under the writer/GC locks after their root is otherwise unreferenced.
  Journal generations and index packs remain marked while any CURRENT, leased,
  retained, backup, checkpoint, import, or fallback root references one of
  their committed prefixes/pages.
- Append, compaction, import, and garbage-collection publication share the
  same in-process mutex and canonical per-source cross-process writer lock.
  Under that lock, an operation verifies the CURRENT pointer, root bundle `R`,
  affected Merkle paths, and affected logical receipts,
  builds a candidate whose `previous_root_sha256` is exactly `R`, rereads
  CURRENT and rejects any mismatch, then atomically renames and fsyncs the
  CURRENT pointer. It must read back CURRENT, the authoritative root bundle,
  and the affected logical receipts before capture may ACK or cleanup may
  become eligible. A
  candidate built from stale `R` can never overwrite `R1`.
  Startup, scrub, import, and restore perform full-tree verification; ordinary
  append/compaction remains logarithmic and never walks all history.
- Superseded objects, logical-manifest files, and unreachable index nodes are
  only eligible for a separate garbage-collection pass after the new root and
  every embedded logical receipt verify, the configured recovery-retention
  interval has elapsed, and a backup/restore policy permits removal. GC first
  emits a canonical exact-path plan, expected hashes/bytes/inodes, authoritative
  root hash, backup receipt, and plan digest. A production retention worker may
  permanently unlink only those regular non-symlink files when an explicitly
  configured policy authorizes that exact digest and root; otherwise an
  operator runs a separate digest-confirmed purge command. It rechecks the
  writer lock, CURRENT root, hashes, paths, and backup receipt immediately
  before unlink and fsyncs every affected directory. Same-filesystem
  quarantine remains an incident-containment option only and is never counted
  as reclaimed bytes/inodes.
- Retention has hard maximum eligible bytes/inodes and maximum age. Breaching a
  limit turns maintenance RED and stops admission before reserve exhaustion;
  the product cannot report bounded archive growth while purge authority is
  absent or an eligible backlog grows without bound.
- Checkpoint sequence evidence and rebuildable hint locators remain
  proportional in bytes to logical archive commits, because original manifest
  IDs are permanent recovery keys, but not proportional in files/inodes.
  Startup requires explicit maximum source
  record rate, minimum group size/maximum group delay, retention horizon, raw
  data budget, receipt-hint/checkpoint budget, and inode budget. It rejects a policy
  whose worst-case commit rate and fixed locator/page overhead exceed any
  budget. Acceptance tests run the configured maximum commit rate over a
  deterministic horizon and require active journal/small-file bounds plus
  index-pack and data-pack growth within the calculated byte/inode envelopes.
- Packing targets 128–512 MiB compressed Parquet objects. Insufficient data at
  a sealed time-partition boundary is one explicit underfilled pack, not one
  object per commit. Hard limits bound input manifests, rows, decoded bytes,
  output bytes, index fanout/depth, and maintenance work per cycle.
- Inspection/status expose logical commits, physical objects, packed and
  uncompacted rows/bytes, underfilled packs, active-tail entries/bytes, index
  nodes/depth, unreachable eligible files, oldest compaction lag, last scrub,
  and recovery duration. Limits fail closed with stable reason codes.

### Non-goals

- Do not change canonical event/block compaction formats.
- Do not infer source qualification or committed trade semantics.
- Do not perform implicit operator-data migration or permanent deletion.
- Do not claim that a short synthetic packing test is a 24-hour production
  soak or backup/restore proof.

## Risks and Failure Modes

- Receipt erosion: deleting an original object before pack reachability and
  row-slice verification would invalidate the checkpoint recovery authority.
- Duplicate or missing replay: a reader that visits both logical and packed
  representations can double count; an interrupted root switch can hide rows.
- Reader/GC race: unlinking objects reachable from a root already selected by
  an inspection or DataFusion query can produce partial or inconsistent reads
  unless the exact root remains leased for the complete query lifetime.
- Lost ACKed data: a compactor publishing from stale root `R` can hide an
  append at `R1` unless every writer shares one lock and uses exact-root CAS
  publication plus readback verification.
- Index corruption or amplification: malformed ranges, excessive fanout,
  page-slice aliasing, cycles, rollback, a hint that disagrees with the
  authoritative sequence tree, or unbounded depth can make recovery unsafe or
  slow.
- Resource exhaustion: compaction can exceed memory/disk reserve by decoding
  too much or temporarily holding both input and output bytes.
- Unsafe migration/GC: path aliasing, symlinks, concurrent writers, stale
  roots, absent purge authority, unverified backup, or broad deletion targets
  can corrupt unrelated data or merely move the growth problem.
- Semantic drift: combining epochs/classes/parser dispositions without
  retaining each logical descriptor can erase source evidence.

## Milestones

### M1. Freeze V3 index and pack contracts

- Goal: make invalid, ambiguous, or unbounded layouts unrepresentable before
  file I/O exists.
- Files / systems:
  - `crates/storage-ports/src/archive.rs`
  - `crates/canonical-archive/src/raw_v3.rs`
  - focused contract/golden tests
- Changes:
  - Add typed packing/retention/capacity policy, sequence-bound logical receipt,
    checkpoint V2 entry, packed range receipt, maintenance statistics, root
    bundle, root lease, bounded authoritative sequence page, rebuildable hint
    page, index-pack, append-only journal generation, and prefix-ref schemas.
  - Freeze canonical JSON bytes and domain-separated digests.
- Verification:
  - RED/GREEN tests for gaps, overlaps, duplicate manifest IDs, invalid row
    slices, duplicate receipt keys, false hint denial, missing sequence
    evidence, excessive fanout/depth,
    mixed source/chain, overflow, stale-root candidates, invalid page slices,
    impossible throughput/horizon budgets, invalid purge plans, and frozen
    golden bytes.
  - Add old-root tests proving a shorter journal prefix remains byte/hash exact
    after later appends and rejects identity, length, count, or prefix-hash
    substitution.
- Expected result: no archive files are published yet; the format contract and
  limits are reviewable.

### M2. Append and read through the bounded V3 index

- Goal: replace full-history catalog rewrites with logarithmic immutable index
  updates while preserving archive-before-ACK receipts.
- Files / systems:
  - `crates/canonical-archive/src/raw_v3.rs`
  - `crates/canonical-archive/src/raw_policy.rs`
  - `crates/canonical-archive/src/lib.rs`
  - `crates/storage-ports/src/archive.rs`
- Changes:
  - Publish immutable logical object/manifest, authoritative sequence journal
    records, fsynced prefix ref, root bundle, and CURRENT pointer in durability
    order; compact bounded pages into immutable index packfiles before journal
    limits are reached without mutating a referenced prefix.
    Publish/rebuild the non-authoritative receipt hint separately.
  - Enforce the shared in-process/cross-process writer lock and exact-root CAS
    with CURRENT/root/receipt readback before returning an append receipt.
  - Verify root/page/pack hashes, page slice bounds, exact sequence coverage,
    hint results against sequence truth, and logical receipts on append, read,
    inspect, epoch lookup, checkpoint recovery, and restart.
  - Add root-specific shared read leases that are owned by every returned
    iterator/query plan and an exclusive GC lease check.
- Verification:
  - Boundary tests for interrupted object/manifest/node/root publication,
    idempotent retry, stale compactor versus append race, concurrent writer
    exclusion, sparse offsets, rotation, quarantine, reader-versus-root-switch/
    GC races, old-reader-versus-journal-append/packing, crash after journal
    append/fsync/root publication, incomplete suffix recovery, journal
    generation rotation, GC denial for every leased older prefix, process-exit
    lease cleanup, stuck-reader health, large history,
    and bounded page reads per sequence lookup; ID-only lookup either
    authenticates a hint or reports that a rebuild is required.
- Expected result: new archives avoid O(n) catalog rewrite and verification;
  V1/V2 behavior is unchanged.

### M3. Pack exact logical ranges idempotently

- Goal: replace many small physical objects with one verified packed object
  without changing any logical receipt or replay byte.
- Files / systems:
  - `crates/canonical-archive/src/raw_v3.rs`
  - `services/hl-analytics/`
  - archive integration tests
- Changes:
  - Select bounded consecutive leaf entries, stream-decode inputs, write one
    packed Parquet object, publish pack manifest, and atomically replace leaf
    entries in a new root.
  - Replace sequence leaves in the candidate root bundle, then rebuild/compact
    affected receipt-hint pages; resolve original manifest IDs plus checkpoint
    sequence ranges through authenticated pack slices.
- Verification:
  - Exact pre/post replay equality; idempotent repeat; mixed epoch/disposition;
    128/512 MiB boundaries with reduced test limits; mutation of every input,
    embedded manifest, slice, output row, data pack, index pack/page, root
    bundle, and CURRENT; crash at every publication boundary; stale-root
    compaction cannot hide a concurrently ACKed append.
- Expected result: current reads visit one authoritative representation; data
  objects and index files/inodes grow with packed ranges, while receipt-hint
  and checkpoint bytes grow at the explicitly calculated bounded rate per
  logical commit.

### M4. Import V2 and reclaim superseded files safely

- Goal: provide a lossless explicit path for retained M3A archives and prove
  checkpoint verification after originals become non-current.
- Files / systems:
  - `crates/canonical-archive/src/raw_v2.rs`
  - `crates/canonical-archive/src/raw_v3.rs`
  - `tools/archive-inspect/`
  - `services/hl-capture/src/auxiliary_checkpoint.rs`
- Changes:
  - Add read-only V2 import planning, full receipt verification, V3 pack/root
    publication, parity report, and explicit operator-approved pointer switch.
  - Derive every original manifest's exact local-sequence range from verified
    V2 manifests and atomically publish
    `auxiliary-archive-checkpoint-v2.json`. Retain checkpoint V1 as
    authoritative until the V2 file fsync, directory fsync, readback,
    root/range/receipt verification, and atomic checkpoint CURRENT switch all
    complete; never reinterpret V1 bytes as if they carried per-manifest
    ranges.
  - Add scoped eligibility plans, root mark sets/leases, backup receipts,
    policy/plan-digest authority, hard eligible-backlog limits, and an fsynced
    deletion journal with per-file planned/unlinked/directory-synced states.
- Verification:
  - Existing auxiliary checkpoint V1 opens unchanged before cutover; generated
    checkpoint V2 has exact contiguous per-manifest ranges and survives crash
    at each publish/switch boundary; imported V3 root replays identically;
    checkpoint IDs+ranges verify through packs after originals are reclaimed;
    alias/symlink/concurrent-writer, active reader lease, stale root,
    missing/wrong backup receipt, wrong plan digest, crash after each unlink or
    directory fsync, verified restore from backup, and partial-import cases
    fail closed.
- Expected result: no implicit migration or data loss; old archives remain a
  safe fallback until the operator approves the verified result.

### M5. Operate, observe, scrub, and bound maintenance

- Goal: make compaction health and failure visible in the running product.
- Files / systems:
  - `services/hl-capture/`
  - `services/hl-analytics/`
  - `tools/archive-inspect/`
  - status/metrics contracts and runbooks
- Changes:
  - Add one owned bounded maintenance task, rate/disk reserve limits, lag and
    growth metrics, stable health reasons, kill switch, graceful shutdown, full
    scrub, and restore report.
- Verification:
  - Maintenance stall/failure degrades health without losing capture; disk
    reserve blocks unsafe output; shutdown joins the task; scrub/restore catch
    missing, duplicate, corrupt, and unreachable artifacts.
- Expected result: operators can distinguish healthy tail, compaction lag,
  corruption, and capacity pressure.

### M6. Retain process and 24-hour evidence

- Goal: prove the real binary remains bounded across source activity, outage,
  rotation, restart, compaction, analytical reads, and restore.
- Files / systems:
  - `tools/ci/capture-e2e.sh`
  - `tools/ci/capture-soak.sh`
  - evidence report schema and runbooks
- Changes:
  - Add reduced-target deterministic E2E and production-target long-soak
    lanes with object/index/inode/recovery high-water marks.
- Verification:
  - Short crash matrix, multi-hour soak, then retained 24-hour soak with at
    least one restart, dependency outage, epoch rotation, scrub, analytical
    query, and restore into a clean directory.
- Expected result: exact evidence distinguishes repo readiness from sustained
  runtime proof.

## Verification

- `cargo +1.97.1 test -p storage-ports -p canonical-archive -p hl-capture -p hl-analytics -p archive-inspect --all-targets --all-features --locked --offline`
- `cargo +1.97.1 clippy -p storage-ports -p canonical-archive -p hl-capture -p hl-analytics -p archive-inspect --all-targets --all-features --locked --offline -- -D warnings`
- `cargo +1.97.1 fmt --all -- --check`
- `git diff --check`
- Manual smoke: run the exact production entrypoint, observe packing metrics,
  verify the archive, stop/restart, replay the same range, and compare hashes.
- Evidence: retain machine-readable input/output object counts and bytes,
  logical rows, index nodes/depth, pack lag, restart/scrub/restore duration,
  resource high-water marks, binary hash, and exact qualification booleans.

## Decision Log

- 2026-08-03: Do not ACK from the hot spool alone. Preserve the reviewed
  archive-before-ACK boundary and solve latency with later physical packing.
- 2026-08-03: Do not mutate frozen V2 manifests or make runtime auto-migration
  destructive. Introduce V3 and an explicit verified import that is reversible
  only until the first V3-only ACK.
- 2026-08-03: Use one authoritative content-addressed sequence tree plus a
  separately rebuildable manifest-hash lookup hint, stored as pages in bounded
  immutable index packfiles with one bounded active journal. V3 receipts and
  checkpoint entries carry exact sequence evidence, so truth lookup stays
  logarithmic without a second authoritative tree or cross-index consensus.
  A flat full-history catalog keeps O(n) rewrite/verify behavior; one
  content-addressed node file per receipt merely moves the inode explosion
  into the index.
- 2026-08-03: Embed original canonical logical-manifest bytes and row slices in
  the pack manifest. Checkpoints keep their original manifest IDs and can be
  verified after physical originals are quarantined.
- 2026-08-03: Serialize append, compaction, import, and GC with the same
  canonical source writer lock and exact-root CAS/readback. Immutable candidate
  files alone do not prevent a stale compactor from hiding an ACKed append.
- 2026-08-03: Separate root publication from garbage collection. A verified
  pack becomes authoritative first; a digest-bound policy or explicit operator
  action plus backup receipt controls later permanent cleanup. Quarantine is
  incident containment, not capacity reclamation.
- 2026-08-03: Pin the exact selected root with an OS-owned shared lease for the
  complete reader/query lifetime. GC never expires a live lock; it fails
  closed and reports the blocking root. CURRENT and writer locks alone do not
  protect a reader already walking an older immutable root.
- 2026-08-03: V2 pointer rollback is reversible only before the first V3-only
  archive ACK. After that boundary, rollback requires a V3-capable binary and
  root/backup restore; the retained V2 root is historical through cutover, not
  an exact current fallback. A separate verified reverse export may be added,
  but is not implied by retaining V2.
- 2026-08-03: Bind each root to an immutable append-only journal-generation
  prefix `(identity, length, record count, prefix hash)`. Later appends may
  extend the file but cannot change bytes authenticated by an older leased
  root; packing and GC wait until no marked root references the generation.
- 2026-08-03: Upgrade auxiliary checkpoint V1 to sequence-aware V2 only through
  verified V2 manifest range derivation and atomic fsync/readback/root
  verification. V1 remains authoritative until the checkpoint CURRENT switch.

## Progress Log

- 2026-08-03: M3A committed at
  `0d8cfaa0eefac982e151cac616c4dd0813101d5f`; reviewer verdict GO for the
  bounded repo-ready slice. Real process evidence remains blocked by host disk
  capacity and unavailable Docker.
- 2026-08-03: Current V2 write amplification and full-history catalog behavior
  confirmed in `raw_v2.rs`; canonical block compaction cannot preserve raw
  per-group spool/epoch receipt semantics without a versioned raw format.
- 2026-08-03: Architecture review held M1 until three P0 invariants were made
  explicit: shared-writer exact-root CAS/readback, actual byte/inode reclamation
  under bounded authorized retention, and a manifest-ID secondary index.
- 2026-08-03: Second review added exact-root reader leases, packed index pages
  with throughput/horizon acceptance envelopes, crash-durable partial-unlink
  recovery, and an honest one-way V2 cutover boundary.
- 2026-08-03: Adopted the reviewer's simpler authority model: manifest-ID
  lookup is a rebuildable hint, while exact sequence evidence in V3 receipts/
  checkpoints authenticates every result against the sole sequence-tree truth.
- 2026-08-03: Third review froze append-only journal prefix identity, atomic
  checkpoint V1-to-V2 upgrade, and affected-path verification for logarithmic
  publication; full-tree work is reserved for bounded startup/scrub.
- 2026-08-03: Next: implement M1 contract types and RED invalid-layout/golden
  tests. No archive write path changes before those tests and review.

## Rollback / Recovery

- If V3 append or packing fails, latch maintenance/capture health as specified,
  retain the last verified CURRENT root, and leave all input V2/V3 logical
  objects and manifests untouched.
- Safe fallback before the first V3 ACK: leave V2 CURRENT authoritative and
  continue read-only verification/export or explicit compatibility capture.
  After the first V3 ACK, retain a V3-capable binary and restore the last
  verified V3 root/backup; never point back to stale V2 as if it were current.
- A failed import never switches the source pointer. A failed or cancelled GC
  pass preserves its fsynced exact deletion journal. Recovery verifies CURRENT,
  leases, the backup receipt, and every remaining path; it either resumes the
  exact authorized purge or restores missing files from the verified backup
  before closing the journal. It never broadens scope or claims an unjournaled
  partial unlink is recoverable.
