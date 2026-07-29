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

### M3 independent-review remediation ExecPlan

**Goal:** Clear the Task 3 HOLD with observable consecutive unresolved metadata
intervals, absence-preserving scale inspection, pre-allocation compound-key
bounds, and complete identity/codec regression evidence.

**Current state:** Commit `7530484` passes its reported gates but rejects a
second hash-only metadata version, exposes absent scales as zero, allocates
oversized compound keys before rejection, and has an incomplete minimum test
matrix. Work remains restricted to the Task 3-owned reducer, exports, tests,
contract, plan, and report.

**Target state:** Strictly increasing metadata versions close either exact or
unresolved open intervals; value-dependent transitions remain suppressed while
unresolved; public applicability getters preserve absence; compound keys reject
oversize before allocation; all record families and identity/collision
boundaries have direct tests. Restore-time semantic scans and authoritative
metadata snapshots remain out of scope.

**Risks and failure modes:** Accidentally re-exposing stale exact values,
accepting overlapping intervals, allocating attacker-sized key buffers,
weakening exact envelope identity, or writing codec tests that exercise only a
single record family.

**Milestones:**

1. Add focused failing interval-chain and scale-absence tests, implement the
   minimal transition/getter changes, and rerun the focused suite.
2. Add failing compound-key boundary and complete identity/codec matrix tests,
   implement checked preflight plus fallible reservation, and rerun the focused
   suite.
3. Update the contract/report, run the focused and combined suites, strict
   Clippy, formatting and diff checks, then create the required new local fix
   commit without amend, rebase, or push.

**Verification:** Run the exact Task 3 focused test, combined
canonical-ledger/replay tests, strict all-target/all-feature Clippy,
`cargo +1.97.1 fmt --all -- --check`, and `git diff --check`.

**Decision log (2026-07-29):** Metadata ordering remains strict lexical version
ordering plus strictly increasing block height. Unresolved intervals may
supersede unresolved intervals, but no exact tick, lot, or scale values are
carried forward.

**Progress log (2026-07-29):** All four review findings are remediated.
Interval-chain and absence-preserving getter REDs were observed, the focused
suite now covers 17 tests, and the combined suites plus strict Clippy pass.
Formatting and diff verification also pass; only the separate fix commit
remains.

**Rollback / recovery:** Keep remediation in a separate commit. If a gate
fails, leave `7530484` and the worktree intact, report the exact failure, and do
not amend, rebase, push, or discard user work.

## Milestone 4: Add replay/checkpoint evidence and soak

**Files:**
- Create: `tools/state-replay/src/market.rs`
- Create: `tools/state-replay/tests/market_e2e.rs`
- Modify: replay CLI/library, `justfile`, README, status, and runbook.

- [x] Write failing runner and CLI tests.
- [x] Generate all typed market events in valid prerequisite order.
- [x] Prove repeated/resumed hash equality, decoded exact state, unresolved
      metadata suppression, malformed transition rollback, unsupported schema
      quarantine, and owner-only evidence.
- [x] Add bounded quick and release-profile soak recipes.
- [x] Run retained evidence, dependency policy, full verification, and OSS
      audit.
- [x] Commit `feat(replay): add canonical market evidence runner`.

## Completion boundary

This plan completes the exact synthetic V1 market-state prerequisite. It does
not qualify deployed node mapping, authoritative metadata snapshots, external
oracle reconciliation, account/funding effects, margin formulas, order books,
Stage 2, or production readiness.
