# Canonical Identity and Block Envelopes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add source-independent stable event identity, a production-safe canonical event constructor, and deterministic canonical block envelopes without claiming source mappings that lack qualified corpora.

**Architecture:** `canonical-events` remains a deterministic synchronous domain crate. Event identity hashes only protocol identity fields with explicit framing; event payload content is hashed separately. Production envelope construction derives the `EventId` and payload hash, while `BlockEnvelope` validates event identity/order and hashes a source-independent canonical projection.

**Tech Stack:** Rust 1.97.1, Rust 2024, BLAKE3, Prost-backed canonical payloads, checked domain newtypes, Cargo test/Clippy, repository `just` gates.

## Global Constraints

- The approved source of truth is `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md` at tag `design-approved-v1.0.0`.
- Rust production code uses Rust 1.97.1, edition 2024, with committed `Cargo.lock` and no unreviewed `unsafe` blocks.
- Canonical accounting and identity use no floating-point values.
- Canonical code is synchronous and deterministic with no network, database, system-clock, random-number, or ingestion-order dependency.
- Stable identity includes chain, block height, transaction identity, canonical event index, event kind, and canonical schema major.
- Variable-length identity fields are length-prefixed before hashing.
- Payload content hash remains separate from `EventId`; equal identity with unequal payload hash is divergence.
- Source identity, source cursor, receive time, ingestion time, and confirmation source never affect `EventId` or canonical block hash.
- Empty committed blocks are valid and retain their block identity.
- No source discriminant is mapped to a canonical payload until its mapping is backed by a qualified fixture.
- Every behavior change follows red-green-refactor and ends with focused and workspace verification.

---

### Task 1: Stable event identity

**Files:**
- Create: `crates/canonical-events/src/event_id.rs`
- Create: `crates/canonical-events/tests/event_id.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Create: `docs/formats/event-id-v1.md`

**Interfaces:**
- Consumes: `ChainId`, `BlockHeight`, `TransactionId`, `EventKind`, and canonical schema major.
- Produces:

```rust
pub struct EventIdentityInput<'a> {
    pub chain_id: &'a ChainId,
    pub block_height: BlockHeight,
    pub transaction_identity: &'a TransactionId,
    pub canonical_event_index: u32,
    pub event_kind: EventKind,
    pub schema_major: u64,
}

pub fn compute_event_id(input: &EventIdentityInput<'_>) -> EventId;
```

- [ ] **Step 1: Write failing identity behavior tests**

Add tests that construct literal domain inputs and assert:

```rust
let first = compute_event_id(&identity("mainnet", 42, "tx-7", 0, EventKind::TradeMatched, 1));
let repeated = compute_event_id(&identity("mainnet", 42, "tx-7", 0, EventKind::TradeMatched, 1));
assert_eq!(first, repeated);
assert!(first.as_str().starts_with("evt_"));
assert_eq!(first.as_str().len(), 68);
```

Use a table that changes exactly one of chain, height, transaction identity,
canonical index, kind, and schema major; every resulting ID must differ from
the baseline. Add the ambiguity regression:

```rust
assert_ne!(
    compute_event_id(&identity("ab", 42, "c", 0, EventKind::TradeMatched, 1)),
    compute_event_id(&identity("a", 42, "bc", 0, EventKind::TradeMatched, 1)),
);
```

The production mutation caught by these tests is omission, reordering, or
unframed concatenation of an identity field.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo +1.97.1 test -p canonical-events --test event_id --locked --offline
```

Expected: compilation fails because `EventIdentityInput` and
`compute_event_id` do not exist.

- [ ] **Step 3: Implement the V1 hash framing**

Use:

```rust
const EVENT_ID_CONTEXT: &str = "hyperliquid-alpha-desk/event-id/v1";

fn hash_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(
        &u64::try_from(bytes.len())
            .expect("identity fields fit in u64")
            .to_be_bytes(),
    );
    hasher.update(bytes);
}
```

Initialize `blake3::Hasher::new_derive_key(EVENT_ID_CONTEXT)`. Hash fields in
this exact order: framed chain bytes, big-endian `u64` block height, framed
transaction bytes, big-endian `u32` canonical event index, framed stable event
kind name, and big-endian `u64` schema major. Return lowercase
`evt_<64-hex-digits>` through `EventId::new`; the generated non-empty,
whitespace-free string is an internal invariant.

- [ ] **Step 4: Verify GREEN and freeze the public contract**

Run the focused test. Record one emitted literal test vector in
`docs/formats/event-id-v1.md`, then add that literal to the test. Document every
field, byte order, framing, domain-separation context, prefix, and the rule that
payload/source/timestamps are excluded.

- [ ] **Step 5: Refactor and verify**

Run:

```bash
cargo +1.97.1 fmt --all -- --check
cargo +1.97.1 clippy -p canonical-events --all-targets --locked --offline -- -D warnings
cargo +1.97.1 test -p canonical-events --test event_id --locked --offline
```

Expected: all commands pass without warnings.

---

### Task 2: Production-safe canonical event construction

**Files:**
- Create: `crates/canonical-events/src/input.rs`
- Create: `crates/canonical-events/tests/input.rs`
- Modify: `crates/canonical-events/src/lib.rs`

**Interfaces:**
- Consumes:

```rust
pub struct CanonicalEventInput {
    pub schema_version: String,
    pub chain_id: ChainId,
    pub block_height: BlockHeight,
    pub block_time: ProtocolTime,
    pub transaction_id: TransactionId,
    pub transaction_index: u32,
    pub canonical_event_index: u32,
    pub market_ids: Vec<MarketId>,
    pub account_ids: Vec<Address>,
    pub source_evidence: Vec<SourceEvidence>,
    pub confirmation_class: ConfirmationClass,
    pub observed_at: KnownTime,
    pub ingested_at: KnownTime,
    pub canonicalized_at: KnownTime,
    pub parser_version: String,
    pub payload: EventPayload,
}
```

- Produces:

```rust
impl SourceEvidence {
    pub fn try_new(
        source_id: SourceId,
        source_version: impl Into<String>,
        source_offset: impl Into<String>,
        content_hash: [u8; 32],
    ) -> Result<Self, ContractError>;
}

impl CanonicalEventEnvelope {
    pub fn from_input(input: CanonicalEventInput) -> Result<Self, ContractError>;
    pub fn expected_event_id(&self) -> EventId;
}
```

- [ ] **Step 1: Write failing construction tests**

Create one literal `CanonicalEventInput` and assert that two inputs differing
only in source evidence and lifecycle timestamps produce the same `EventId` and
payload hash. Assert that changing the payload preserves `EventId` but changes
payload hash. Assert that changing the canonical event index changes
`EventId`.

Add boundary tests proving `from_input` rejects:

- empty source evidence;
- blank or padded source version/offset;
- unsupported or malformed canonical schema version;
- blank or padded parser version;
- `observed_at > ingested_at`;
- `ingested_at > canonicalized_at`.

The production mutations caught are caller-supplied identity, missing evidence,
and time-order corruption.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo +1.97.1 test -p canonical-events --test input --locked --offline
```

Expected: compilation fails because `CanonicalEventInput`,
`SourceEvidence::try_new`, and `CanonicalEventEnvelope::from_input` do not
exist.

- [ ] **Step 3: Implement validated construction**

Move only the input DTO into `input.rs`; keep envelope encoding/decoding in
`lib.rs`. `from_input` must:

1. validate semantic schema major 1;
2. require at least one `SourceEvidence`;
3. require `observed_at <= ingested_at <= canonicalized_at`;
4. validate the parser version with the existing required-string rule;
5. encode the typed payload and compute its BLAKE3 content hash;
6. derive `EventId` with `compute_event_id`, using the payload’s `EventKind`;
7. retain exact encoded payload bytes.

Add accessors required by the block boundary:

```rust
pub fn chain_id(&self) -> &ChainId;
pub fn transaction_id(&self) -> &TransactionId;
pub const fn transaction_index(&self) -> u32;
pub const fn canonical_event_index(&self) -> u32;
pub fn source_evidence(&self) -> &[SourceEvidence];
pub fn parser_version(&self) -> &str;
```

Keep the existing fixture convenience constructor compatible, but document that
production canonicalization must use `from_input`.

- [ ] **Step 4: Verify GREEN and existing compatibility**

Run:

```bash
cargo +1.97.1 test -p canonical-events --test input --locked --offline
cargo +1.97.1 test -p canonical-events --test envelope --locked --offline
cargo +1.97.1 test -p canonical-events --test payload --locked --offline
```

Expected: new and existing envelope/payload tests pass.

---

### Task 3: Deterministic canonical block envelope

**Files:**
- Create: `crates/canonical-events/src/block.rs`
- Create: `crates/canonical-events/tests/block.rs`
- Modify: `crates/canonical-events/src/lib.rs`
- Create: `docs/formats/canonical-block-v1.md`

**Interfaces:**
- Consumes canonical events and per-source raw block hashes.
- Produces:

```rust
pub struct BlockEnvelope {
    chain_id: ChainId,
    block_height: BlockHeight,
    block_time: ProtocolTime,
    confirmation_class: ConfirmationClass,
    events: Vec<CanonicalEventEnvelope>,
    source_block_hashes: BTreeMap<SourceId, [u8; 32]>,
    canonical_block_hash: [u8; 32],
}

impl BlockEnvelope {
    pub fn try_new(
        chain_id: ChainId,
        block_height: BlockHeight,
        block_time: ProtocolTime,
        confirmation_class: ConfirmationClass,
        events: Vec<CanonicalEventEnvelope>,
        source_block_hashes: BTreeMap<SourceId, [u8; 32]>,
    ) -> Result<Self, BlockError>;
}
```

- [ ] **Step 1: Write failing block-invariant tests**

Build events through `CanonicalEventEnvelope::from_input`. Tests must prove:

- an empty committed block with one source hash is valid;
- primary and independent observations of the same canonical content have the
  same canonical block hash despite different source evidence and confirmation
  class;
- the block rejects empty source hashes, mixed chain, mixed height, mixed block
  time, mixed confirmation class, duplicate event IDs, invalid event IDs, a
  first event index other than zero, repeated/decreasing order, and gaps within
  one transaction’s canonical event indices;
- transaction indices may skip when intervening transactions emit no canonical
  event, but a new transaction’s first canonical event index must be zero.

The production mutations caught are nondeterministic block identity, accidental
source coupling, and acceptance of ambiguous ordering.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo +1.97.1 test -p canonical-events --test block --locked --offline
```

Expected: compilation fails because `BlockEnvelope` and `BlockError` do not
exist.

- [ ] **Step 3: Implement fail-closed block validation**

`BlockError` is a typed `thiserror` enum with stable `reason_code()` values for
every rejection category. Validation must compare each event’s expected
identity, chain, height, time, confirmation class, and `(transaction_index,
canonical_event_index)` order before computing a hash.

Compute the source-independent hash using
`blake3::Hasher::new_derive_key("hyperliquid-alpha-desk/canonical-block/v1")`
over:

1. framed chain bytes;
2. big-endian block height;
3. big-endian block time microseconds;
4. big-endian event count;
5. for every event in validated order: transaction index, canonical event
   index, framed event ID, framed event-kind stable name, and the 32-byte
   payload hash.

Do not include confirmation class, source hashes/evidence, observed/ingested/
canonicalized times, parser version, or encoded source bytes.

The implementation was subsequently hardened before archive adoption to bind
the canonical schema version plus ordered market and account routing metadata.
Those fields affect downstream routing and therefore cannot be excluded from
the canonical content projection. Source, confirmation, lifecycle, parser, and
raw-byte evidence remain excluded.

- [ ] **Step 4: Verify GREEN and document the projection**

Run:

```bash
cargo +1.97.1 test -p canonical-events --test block --locked --offline
cargo +1.97.1 test -p canonical-events --all-targets --locked --offline
```

Document the exact projection, empty-block behavior, ordering invariants, and
source-independent exclusions in `docs/formats/canonical-block-v1.md`.

---

### Task 4: Full local verification and status update

**Files:**
- Modify: `README.md`
- Modify: `CHANGELOG.md`
- Modify: `docs/STATUS.md`
- Modify: `docs/ROADMAP.md`
- Modify: `tools/ci/check-generated.sh`

**Interfaces:**
- Consumes the verified identity/input/block implementation.
- Produces truthful repository status and deterministic clean-tree checks.

- [ ] **Step 1: Extend generated verification**

Add locked/offline focused runs for `event_id`, `input`, and `block` to the
detached-tree generated check. Do not mark source-to-canonical parsing,
sequencing, archive, or runtime as implemented.

- [ ] **Step 2: Run focused and workspace checks**

Run:

```bash
git diff --check
cargo +1.97.1 fmt --all -- --check
cargo +1.97.1 clippy -p canonical-events --all-targets --locked --offline -- -D warnings
cargo +1.97.1 test -p canonical-events --all-targets --locked --offline
just verify
just oss-audit
```

Expected: all checks pass; the OSS audit reports `PASS`.

- [ ] **Step 3: Commit the implementation slice**

Review `git diff --stat`, `git diff --check`, and the intentional file list,
then commit:

```bash
git add crates/canonical-events docs/formats/event-id-v1.md docs/formats/canonical-block-v1.md docs/STATUS.md docs/ROADMAP.md README.md CHANGELOG.md tools/ci/check-generated.sh
git commit -m "feat(canonical): add stable event and block identity"
```

- [ ] **Step 4: Verify the exact committed tree**

Run:

```bash
just generated
just oss-audit
test -z "$(git status --porcelain)"
git log -1 --oneline
```

Expected: generated verification and OSS audit pass at the committed SHA, and
the isolated worktree is clean.

## Self-review

- Spec coverage: this plan implements only the unblocked identity and block
  foundation of Stage 1 Task 5. Exhaustive source mappings, upcasters,
  `canonical-inspect`, cross-machine source corpus comparison, sequencer,
  archive, bus, and runtime remain in their existing ordered tasks.
- Placeholder scan: the plan contains no unresolved implementation choice,
  placeholder value, or unspecified validation category.
- Type consistency: `canonical_event_index` is the existing envelope
  `event_index` field with a clearer domain accessor; no wire-schema field is
  renamed in this compatible slice.
