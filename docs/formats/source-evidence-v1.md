# Source Evidence V1

`SourceEvidence` links a canonical event to the immutable parent source record
that produced it.

The V1 fields are:

- `source_id`: validated identity of the configured source;
- `source_version`: immutable source contract/parser version;
- `source_offset`: parent source cursor or fixture offset;
- `content_hash`: 32-byte BLAKE3 hash of the complete parent source record; and
- `source_event_index`: optional zero-based position of the mapped item inside
  the parent record.

`source_event_index` uses explicit Protobuf presence. `None` means that the
source record itself maps as one indivisible item. `Some(0)` means the first
item in a one-to-many or batch mapping; it is not equivalent to absence.

The sub-index never replaces or modifies `source_offset`. Every canonical event
from one parent record therefore retains the same source identity, version,
offset, and content hash, while its sub-index identifies the deterministic
mapping position. Mapping order must follow the versioned source contract and
must not use observation or ingestion time as a tie-breaker.

Source evidence is excluded from canonical event and block identity
projections. It remains required provenance for reconciliation, quarantine,
incident reproduction, and archive audit.

## Current committed-node mapping boundary

The `node-v1` committed mapper currently accepts transaction-block records
whose `abci_block` has a contiguous `round`/`parent_round`, a valid protocol
timestamp, and exactly one unambiguous `signed_action_bundles` array. It maps
only an empty bundle array to an empty committed `BlockEnvelope`, retaining the
complete source-record BLAKE3 hash as block evidence.

Any action-bearing committed block fails closed with
`canonical_mapping.unsupported_committed_actions`. This is intentional: the
complete signed-action and response corpus has not yet been qualified into a
versioned canonical mapping contract. Neither fixture replay nor an empty-block
mapping qualifies the live action-bearing source path for production.

Committed node-directory configuration requires the operator assertion
`replica_cmds_style = "actions-and-responses"` and rejects the node's
action-only profiles. That prevents knowingly deploying an input which cannot
describe action outcomes, but it does not attest the running node flags or
qualify an inferred response schema. Those require retained build/command
evidence plus redistribution-reviewed operator recordings.
