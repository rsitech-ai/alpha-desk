# Committed source failover state V1

`hl.capture.failover.v1` is the private, create-once local record that prevents
capture from silently returning to its primary committed source after a
one-way failover.

The configured location is `runtime.failover_state_path`. It is a local state
file, not a publication contract and not a substitute for the retained source
spools.

## Stored fields

| Field | Meaning |
| --- | --- |
| `schema_version` | Exactly `hl.capture.failover.v1` |
| `chain_id` | Canonical chain identity |
| `primary_source_id` | Configured locally verified committed source |
| `independent_source_id` | Configured independent committed source |
| `failover_height` | First canonical height selected from the independent source |
| `reason` | Stable reason; V1 permits `primary-range-unavailable` |
| `decision_hash_blake3` | Lowercase BLAKE3 digest of the versioned decision material |

Unknown fields, malformed typed identifiers, an invalid digest, source
aliasing, and a chain or source-role mismatch fail startup closed.

## Persistence and restart rules

- The file is created with mode `0600`.
- The serialized decision is written to a private temporary file, fsynced,
  installed without replacing an existing destination, and followed by a
  parent-directory fsync.
- Recording the identical decision is idempotent.
- A different decision at the same path is a conflict; the existing record is
  never rewritten.
- Symlinked state files or parent path components are rejected.
- Once present, the record selects the independent source from
  `failover_height` onward. Restart must validate and honor it before canonical
  draining resumes.
- Automatic failback is not part of V1. It requires an explicit overlap
  reconciliation design and a new operator-approved transition.

## Recovery

Do not edit, delete, or regenerate this file to recover capture. Preserve the
file, both source spools, the raw archive, the canonical archive, the
PostgreSQL progress cursor, and status evidence. A checksum or topology error
is an operator investigation boundary, not permission to return to primary.

The record proves a local software decision only. Synthetic primary and
independent directories do not prove that the sources are separately operated
or qualify the runtime as live-source production evidence.
