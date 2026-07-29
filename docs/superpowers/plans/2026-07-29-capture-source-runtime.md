# Committed Source Runtime Implementation Plan

**Goal:** Replace the `hl-capture run` placeholder with a recoverable,
fail-closed primary-node path that durably spools raw observations before
canonical mapping and publication.

**Authority boundary:** This plan implements local read-only acquisition. It
does not add trading, signing, private-key access, publication to a remote
repository, or claims of live-source qualification. Action-bearing committed
records remain rejected until their complete source semantics are qualified.

**Architecture:** Each configured source owns a separate spool subdirectory and
one hash-chained segment sequence. Startup validates all closed evidence,
repairs only an incomplete open tail, replays durable spool records beyond the
canonical database cursor, and then resumes the source adapter from the final
durable raw cursor. The live loop fsyncs the raw observation before acknowledging
the adapter, maps and sequences it, and delegates archive/publication/cursor
commit to the existing `CaptureCoordinator`.

## Slice 1: Recoverable source journal

- Add a bounded spool-directory inventory exposing ordered segment paths, the
  open tail, next sequence, and manifest-chain tip.
- Add a journal that recovers/resumes the open tail or creates the next segment.
- Replay every verified record in cursor order.
- Rotate by configured byte and elapsed-time limits, close with the previous
  manifest hash, and create the next segment.
- Test crash-tail recovery, restart append, rotation, and unsafe/multiple-open
  failure modes.

## Slice 2: Committed observation processor

- Convert a spooled observation into the versioned node record.
- Map only qualified committed semantics.
- Build a source-admission-bound sequencer candidate.
- Execute all sequencer decisions explicitly; commit through
  `CaptureCoordinator`, and fail closed on gap/quarantine/unimplemented
  recovery decisions.
- Test the exact ordering: no adapter acknowledgement before fsync, no
  coordinator call before a successful mapping, and restart replay only beyond
  the durable canonical cursor.

## Slice 3: Owned source task and CLI

- Construct the node block-directory adapter from validated configuration,
  including an explicit source software/version identity.
- Start the source task under the existing cancellation and bounded-shutdown
  owner.
- Persist malformed raw evidence to a durable quarantine boundary before
  acknowledging it; never skip an unknown record.
- Wire `hl-capture run` and stable reason-coded failures.
- Add a self-contained source-to-spool-to-archive-to-JetStream-to-PostgreSQL
  E2E that restarts from the same durable state.

## Completion checks

- Focused red-green tests for each behavior.
- Rustfmt and strict Clippy for touched crates.
- All `canonical-events` and `hl-capture` tests.
- Existing owned-runtime E2E and a new committed-source E2E.
- Architecture, dependency, secret, and OSS-policy audits.
- Documentation and runbooks name the exact live qualification boundary and
  evidence paths.
