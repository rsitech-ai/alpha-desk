# Task 3 report — versioned market registry

## Status

Complete and locally verified. The canonical Stage 2 V1 prerequisite reducer
owns the exact twelve market event kinds at schema `1.0.0` under reducer version
`hyperliquid-alpha-desk-canonical-market@1.0.0`.

## Commit

- `feat(state): add versioned dynamic market registry` (this report is included
  in that local commit; the resolved SHA is returned to the orchestrator after
  commit creation)

## Exact tests

Observed RED:

```text
cargo +1.97.1 test -p canonical-ledger --test market_state --locked --offline
error[E0432]: unresolved imports for the absent market reducer and record APIs
```

Final verification:

```text
cargo +1.97.1 test -p canonical-ledger --test market_state --locked --offline
13 passed; 0 failed

cargo +1.97.1 test -p canonical-ledger -p replay-engine --locked --offline
all canonical-ledger, replay-engine, and doc tests passed

cargo +1.97.1 clippy -p canonical-ledger -p replay-engine --all-targets --all-features --locked --offline -- -D warnings
passed with no warnings

git diff --check
passed
```

## Key design choices

- Every accepted owned event writes one immutable event-ID-bound fact.
- DEX, asset context, market, metadata version, and outcome identities use
  bounded length-framed state keys and collision rejection.
- `MarketCreated` requires an existing DEX and two distinct existing assets,
  installs exact `creation@1.0.0` metadata, and derives scales only from its
  canonical tick and lot values.
- Hash-only metadata changes close the prior interval at the previous block,
  create a non-overlapping unresolved interval, and remove exact tick, lot, and
  scale applicability. Cap, margin-table, oracle, funding, and outcome
  resolution then fail with `market_state.metadata_unresolved`.
- Halt/resume and outcome resolution are explicit default-deny state machines.
  Oracle and funding times never regress, and later cap/table changes must bind
  their exact current predecessor.
- Because `MarketCreated` carries neither an open-interest cap nor margin-table
  hash, the first corresponding change event establishes its asserted
  predecessor and stores the new current value; every later event must match
  the stored current value exactly.
- All record codecs are strict field-ordered canonical JSON, deny unknown
  fields, enforce a 16 KiB bound, and provide key-bound decoding without
  exposing mutable state.

## Remaining concerns

- This is deterministic synthetic canonical-state evidence only. Deployed node
  mapping, authoritative metadata snapshots that can resolve a hash-only
  version, external oracle reconciliation, account/position effects, margin
  formulas, order books, and Stage 2 qualification remain outside M3.
- The first cap/table transition bootstrap rule is necessary because the V1
  `MarketCreated` contract omits those predecessor values; a future schema that
  supplies an authoritative creation snapshot should replace that bootstrap
  with exact creation-time state.
