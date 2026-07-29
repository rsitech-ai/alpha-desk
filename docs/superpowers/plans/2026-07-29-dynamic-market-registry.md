# Alpha Desk Dynamic Market Registry Implementation Plan

> **Execution:** Follow `superpowers:executing-plans` and
> `superpowers:test-driven-development`. Observe each failing test before
> implementation and keep canonical state synchronous, deterministic, bounded,
> and storage-neutral.

**Goal:** Type the complete V1 market-event family and reconstruct a
point-in-time market registry that supplies exact metadata prerequisites to
account, position, funding, margin, book, and intelligence reducers.

**Architecture:** `api-contracts` owns strict Protobuf wire codecs;
`canonical-events` owns validated domain payloads while retaining the enclosing
wire bytes; `canonical-ledger` owns immutable facts, current market state,
metadata-version intervals, and default-deny transitions. A bounded
`state-replay market-e2e` path proves deterministic rebuild and checkpoint
resume without claiming deployed-source or Stage 2 qualification.

**Critical boundary:** `MarketMetadataChanged` carries only a version and hash.
It cannot prove new tick size, lot size, decimal scales, asset links, or margin
rules. The reducer records the change and marks exact metadata unresolved until
an authoritative metadata snapshot supplies those fields. It must never copy
forward old values as if they belonged to the new hash.

---

## Milestone 1: Type registry identity and metadata contracts

**Files:**
- Modify: `crates/api-contracts/src/lib.rs`
- Modify: `crates/api-contracts/tests/payload_codec.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Modify: `crates/canonical-events/tests/payload.rs`

**Events:**
- `DexCreated`
- `AssetContextUpdated`
- `MarketCreated`
- `MarketMetadataChanged`

- [x] Write strict wire round-trip and malformed-boundary tests.
- [x] Observe the focused compile/test failure.
- [x] Add bounded wire structs/codecs and typed canonical payloads.
- [x] Require valid IDs/addresses, positive tick/lot values, exact 32-byte
      content hashes at the domain boundary, and bounded control-free
      names/versions.
- [x] Prove direct typed construction cannot bypass encode-time validation.
- [x] Run affected crate suites, strict Clippy, and OSS audit.
- [x] Commit `feat(events): type canonical market metadata payloads`.

## Milestone 2: Type status, valuation, and outcome contracts

**Files:**
- Modify the same contract/event files and tests.

**Events:**
- `MarketHalted`
- `MarketResumed`
- `OpenInterestCapChanged`
- `MarginTableChanged`
- `OracleUpdated`
- `FundingRateUpdated`
- `OutcomeCreated`
- `OutcomeResolved`

- [x] Write strict red tests for all values and transitions.
- [x] Add positive/nonnegative fixed-point validation, bounded reasons/sources,
      nonnegative protocol timestamps, exact previous/new values, and immutable
      outcome identity.
- [x] Preserve enclosing unknown fields on decode/re-encode.
- [x] Run affected crate suites, strict Clippy, and OSS audit.
- [x] Commit `feat(events): type canonical market state payloads`.

## Milestone 3: Implement the versioned market registry

**Files:**
- Create: `crates/canonical-ledger/src/market.rs`
- Create: `crates/canonical-ledger/tests/market_state.rs`
- Modify: `crates/canonical-ledger/src/lib.rs`
- Modify: `docs/contracts/deterministic-state-v1.md`

- [x] Write failing table-driven lifecycle, point-in-time, key/codec, and
      whole-block rollback tests.
- [x] Add reducer version
      `hyperliquid-alpha-desk-canonical-market@1.0.0`.
- [x] Store immutable event facts, current DEX/asset/market/outcome state, and
      non-overlapping metadata-version intervals.
- [x] Derive price/quantity scales only from the canonical tick/lot values that
      created the market; reject invalid transitions and unknown prerequisites.
- [x] On `MarketMetadataChanged`, close the prior exact interval, record the new
      version/hash as unresolved, and suppress value-dependent updates until
      authoritative metadata is supplied.
- [x] Default-deny halt/resume, cap, margin table, oracle, funding, and outcome
      transitions; keep all arithmetic checked and fixed-point.
- [x] Run canonical-ledger/replay suites and strict Clippy.
- [x] Commit `feat(state): add versioned dynamic market registry`.

**Progress (2026-07-29):** M3 is implemented and locally verified with 13
focused market-state tests, the combined canonical-ledger/replay suites, and
strict all-target/all-feature Clippy. The local M3 commit is recorded in the
task report.

## Milestone 4: Add replay/checkpoint evidence and soak

**Files:**
- Create: `tools/state-replay/src/market.rs`
- Create: `tools/state-replay/tests/market_e2e.rs`
- Modify: replay CLI/library, `justfile`, README, status, and runbook.

- [ ] Write failing runner and CLI tests.
- [ ] Generate all typed market events in valid prerequisite order.
- [ ] Prove repeated/resumed hash equality, decoded exact state, unresolved
      metadata suppression, malformed transition rollback, unsupported schema
      quarantine, and owner-only evidence.
- [ ] Add bounded quick and release-profile soak recipes.
- [ ] Run retained evidence, dependency policy, full verification, and OSS
      audit.
- [ ] Commit `feat(replay): add canonical market evidence runner`.

## Completion boundary

This plan completes the exact synthetic V1 market-state prerequisite. It does
not qualify deployed node mapping, authoritative metadata snapshots, external
oracle reconciliation, account/funding effects, margin formulas, order books,
Stage 2, or production readiness.
