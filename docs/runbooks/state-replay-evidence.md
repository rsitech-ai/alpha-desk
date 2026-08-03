# State Replay Evidence

`state-replay fixture-e2e` is the first runnable Stage 2 evidence path. It is a
bounded synthetic fixture runner, not a live-source or production-state
qualification.

## Quick evidence run

From the repository root:

```bash
just state-replay-e2e
```

The recipe creates a new private directory under
`target/evidence/state-replay/` and prints the resulting `report.json` path. It
never reuses or overwrites an existing evidence directory.

The run:

1. creates a deterministic local canonical Parquet archive;
2. rebuilds its empty committed-block range independently for the requested
   number of iterations;
3. requires every final state hash and full replay receipt hash to match;
4. replays a prefix, publishes a private immutable local checkpoint, reloads
   it under exact archive/schema/reducer compatibility, and resumes the suffix;
5. requires resumed and uninterrupted state hashes to match; and
6. appends one typed trade block and requires the watermark-only production
   reducer to quarantine it without advancing state.

The report explicitly records:

- `evidence_class = "synthetic_fixture"`;
- `stage_2_qualified = false`;
- `live_source_qualified = false`;
- deterministic full and resumed state/receipt hashes;
- the content-derived checkpoint ID;
- iteration count and replay duration; and
- poison-block reason, progress, and before/after state hashes.

## Longer soak

```bash
just state-replay-soak
```

Defaults are 1,000 archived blocks, a checkpoint after 500 blocks, and 100
independent full replays. Override the bounded parameters when needed:

```bash
just state-replay-soak 2000 1000 500
```

The runner rejects fewer than two blocks, a checkpoint outside the range, zero
iterations, more than 100,000 blocks or iterations, or more than 100,000,000
total replayed blocks across the repeated and checkpoint-equivalence passes.
Disk use is driven mainly by the one-time Parquet archive; runtime grows
approximately with `blocks * (iterations + 1)`.

## Canonical trade evidence

Run the exact canonical trade-fact reducer through the same immutable archive
and private checkpoint path:

```bash
just state-replay-trade-e2e
```

The command writes a new private directory under
`target/evidence/state-replay-trade/`. Each generated committed block contains
one exact `TradeMatched@1.0.0` event. Blocks alternate between enriched
participant-bearing trades and participant-free legacy trades, starting with
an enriched trade. The composite
`hyperliquid-alpha-desk-canonical-trade-set@2.0.0` reducer preserves the frozen
V1 facts for every trade and adds V2 participant-anchor facts only for enriched
trades. The deterministic archive producer identity is
`state-replay-trade-e2e-v2`, and generated envelopes bind parser version
`state-replay-trade-fixture-v2`; both identities are recorded in the V2 report.
The run:

1. requires all independent rebuild state and receipt hashes to match;
2. publishes, verifies, loads, and resumes a prefix checkpoint;
3. publishes genuine direct V1 and direct V2 component checkpoint artifacts,
   then proves the local checkpoint store rejects both under composite
   compatibility with the exact underlying incompatibility;
4. decodes every V1 fact and reconciliation, plus every enriched V2 trade,
   buyer/seller participant fact, and position-symmetry reconciliation;
5. requires exact per-namespace cardinality, opposite signed effects, and a
   passed reconciliation for every applicable generated trade;
6. requires a malformed trade to fail with
   `trade_state.invalid_trade_id`/`ledger.reducer_failed` without state change;
   and
7. requires an unsupported trade schema to fail with
   `ledger.unsupported_event` without state change.

The report declares
`evidence_class = "synthetic_canonical_trade"`,
`state_semantics = "canonical_trade_facts_and_exact_participant_anchors"`,
`source_qualification = "synthetic_unassessed"`, Stage 1 and Stage 2 false,
live-source qualification false, and account/order/position qualification
false. V1 participant indices remain stable evidence ordinals with no semantic
role claim. V2 ordinal 0 is the source-declared buyer with positive quantity
effect and ordinal 1 is the source-declared seller with negative quantity
effect. These are exact synthetic contract facts, not deployed-source or
account-position qualification.

For a longer bounded run:

```bash
just state-replay-trade-soak
```

Defaults are 1,000 trade blocks, a checkpoint after 500 blocks, and 100
independent rebuilds. Override them with the same positional
`blocks checkpoint_after iterations` arguments used by
`state-replay-soak`. Preserve the entire evidence directory, including both
rejection archives and the checkpoint generation.

## Canonical order evidence

Run the exact canonical order lifecycle through the immutable archive and
private checkpoint path:

```bash
just state-replay-order-e2e
```

The command writes a new private directory under
`target/evidence/state-replay-order/`. Alternating generated blocks cover
acceptance, resting, modification, partial and terminal fill, cancellation,
and rejection. The run requires:

1. identical state and receipt hashes across independent rebuilds;
2. a published, verified, loaded, and resumed prefix checkpoint;
3. strict decoding and exact cardinality for every immutable fact, current
   order, and transition assessment;
4. exact filled, cancelled, and fact-only rejection counts;
5. a late overfill to fail with
   `order_state.overfill`/`ledger.reducer_failed` without state change; and
6. schema `1.1.0` to fail with `ledger.unsupported_event` without state change.

The report sets `synthetic_order_contract_proven = true` and keeps Stage 1,
Stage 2, deployed/live source, position, margin, and execution qualification
false. It is generated canonical-event evidence, not proof that a deployed
Hyperliquid source emits these semantics.

For a longer bounded run:

```bash
just state-replay-order-soak
```

Defaults are 1,000 order blocks, a checkpoint after 500 blocks, and 100
independent rebuilds. Preserve the entire evidence directory, including both
rejection archives and the checkpoint generation.

## Canonical market evidence

Run the exact canonical market registry through the immutable archive and
private checkpoint path:

```bash
just state-replay-market-e2e
```

The command writes a new private directory under
`target/evidence/state-replay-market/`. Generated V1 events create the DEX,
base and quote asset contexts, market, and outcome before applying valuation,
cap/table, halt/resume, and outcome transitions. The final valid block changes
the market to a hash-only metadata version, so the primary replay range and
the resumed checkpoint suffix both cross the exact-to-unresolved boundary.
The run requires at least two independent rebuilds and:

1. identical unresolved final-state and full replay-receipt hashes across
   independent rebuilds;
2. a published, verified, loaded, and resumed prefix checkpoint with the same
   unresolved final-state hash after its suffix crosses the metadata change;
3. strict decoding and exact cardinality for every market fact, DEX, asset,
   current market, both metadata versions, and outcome namespace after every
   independent replay and the resumed path;
4. one closed exact prior interval and one open unresolved current interval,
   absent exact-value getters, exact status/resolution counts, and a
   deterministic unresolved market/resolved-outcome sample;
5. a later oracle value suppressed with `market_state.metadata_unresolved` and
   no block effects;
6. a valid oracle update followed by an invalid resume to roll back the whole
   late block with `market_state.invalid_status_transition`; and
7. schema `1.1.0` to quarantine with `ledger.unsupported_event` and no state
   change.

The report schema is
`hyperliquid-alpha-desk/state-replay-market-e2e-report/v1`. It sets only
`synthetic_market_contract_proven = true`; Stage 1, Stage 2, deployed/live
source, authoritative metadata, external oracle reconciliation, account,
position, margin, book, signal, and execution qualification remain false.
Evidence is generated from synthetic canonical events, with source
qualification explicitly `synthetic_unassessed`.

For a longer bounded release-profile run:

```bash
just state-replay-market-soak
```

Defaults are 1,000 market blocks, a checkpoint after 500 blocks, and 100
independent rebuilds. Override them with positional
`blocks checkpoint_after iterations` arguments. Preserve the entire private
evidence directory, including the primary, malformed, and unsupported
archives, metadata-suppression block, checkpoint generations, and report.

## Canonical account evidence

Run the bounded synthetic account-flow/composite proof with:

```bash
just state-replay-account-e2e 30 12 4
```

The runner retains a new `0700` directory below
`target/evidence/state-replay-account/`, with `0600` reports and immutable
archive/checkpoint material. It refuses existing or unsafe output paths. It
requires independent full replays with identical final state and full receipt
hashes, resumes a checkpoint suffix crossing vault and account/margin-mode
flows, counts the account-flow/relation/mode namespaces, denies missing typed
asset/market prerequisites, rolls back a late cross-component failure, and
denies schema `1.1.0`.

The report is synthetic canonical account evidence only:

```text
evidence_class = synthetic_canonical_account
state_semantics = exact_observed_account_flows_relations_and_modes
source_qualification = synthetic_unassessed
```

Only `synthetic_account_flow_contract_proven` is true. Position, episode,
liquidation, settlement, funding-attribution, authoritative opening-balance,
venue-reconciliation, TWAP-completeness, backstop-cost-basis, margin-model,
liquidation-price, book, signal, execution, Stage 1/2, deployed-source, and
live-source qualification remain false. Task 8 owns the excluded risk and
position scenarios.

## Canonical position evidence

Run the bounded synthetic composite-position proof with:

```bash
just state-replay-position-e2e 30 12 4
```

The runner creates a private `0700` directory below
`target/evidence/state-replay-position/` and hardens every retained file to
`0600`. It refuses existing or unsafe output. Configuration must leave seven
suffix blocks after the opening-trade checkpoint. The report retains repeated
full-range receipt equality, byte-identical checkpoint load, segmented suffix
receipts, exact final state equality, decoded semantic checks, the rejected
`-2.75` settlement-PnL variant, and duplicate-trade/start-anchor/schema atomic
rejection reports.

The proof is synthetic only. Preserve the full evidence directory, including
the main archive, checkpoint generations, semantic variant, three rejection
archives, and report. A positive report does not qualify deployed/live source,
authoritative balances or positions, venue reconciliation, protocol entry
price or closed-PnL parity, fee/TWAP completeness, backstop basis, margin,
liquidation price, book, signal, execution, Stage 1/2, or a live product.

## Existing operator archive

Run the same deterministic rebuild and checkpoint-resume proof against an
existing local canonical archive:

```bash
just state-replay-archive-e2e \
  /absolute/path/to/archive \
  mainnet \
  1000000 \
  1000999 \
  1000499 \
  3
```

For a longer release-profile run, use the same arguments with:

```bash
just state-replay-archive-soak \
  /absolute/path/to/archive \
  mainnet \
  1000000 \
  1000999 \
  1000499 \
  100
```

Before creating the private evidence directory, this mode:

1. requires an existing, non-symlink archive directory;
2. verifies the selected range through the current archive catalog;
3. freezes the range into ordered immutable manifest IDs and hashes;
4. requires contiguous manifest ranges, one chain, and one canonical schema
   fingerprint;
5. requires the checkpoint height to be an exact manifest boundary; and
6. rejects evidence output inside or above the archive.

Replay then reads only the frozen immutable manifests. It does not append,
compact, repair, or otherwise mutate the operator archive. The evidence
directory contains only the private checkpoint generations and report.

The operator report records every immutable manifest ID, manifest hash, range,
and row count. It also declares:

- `evidence_class = "operator_archive"`;
- `state_semantics = "watermark_only"`;
- `source_qualification = "unassessed"`;
- `stage_2_qualified = false`; and
- `live_source_qualified = false`.

An action-bearing block is expected to stop with
`replay.block_quarantined`/`ledger.unsupported_event` until its reducer is
qualified. Do not relabel such a stop as archive corruption.

## Interpreting the result

A successful fixture report is `runtime-proven:synthetic` evidence for
deterministic serial replay, local checkpoint resume, and poison-block
atomicity. A successful trade report additionally proves the exact implemented
frozen V1 trade-fact contract and enriched V2 buyer/seller anchor and signed
effect reconciliation contracts against generated canonical events. Neither
proves RocksDB durability, deployed Hyperliquid source compatibility, external
account/book reconciliation, a current position reducer, complete
action-bearing state, service readiness, or the Stage 2 gate.

Retain the complete evidence directory when comparing runs. The archive,
checkpoint generation, and report belong together; do not copy only the JSON
and call it reproducible evidence.

A successful operator-archive report is stronger evidence about that exact
archive range, but it is still not live-source qualification: the tool does not
know how the archive was captured, reviewed, or reconciled. Preserve the
operator archive immutably alongside the report and independently verify it
with `just archive-verify /absolute/path/to/archive`.
