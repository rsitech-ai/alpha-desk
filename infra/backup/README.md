# Stage 0 backup and recovery policy scaffold

Status: **policy only; no backup automation, backup artifact, restore, or
runtime proof exists in Stage 0.**

This directory intentionally contains no timer, cron entry, credentials,
destructive command, or production target. Every future mutating procedure is
a **later operator procedure — do not execute in Stage 0**.

## Ownership and protected data

The SRE/security owner is accountable for backup transport, encryption, key
separation, retention, drills, and evidence. The platform/data owner validates
reconstructed data and deterministic checkpoint hashes. Neither role may
declare recovery successful from a copy command alone.

Protected sets are:

- committed raw observations and immutable Parquet archive, replicated to a
  second operator-controlled site;
- PostgreSQL continuous WAL archive and encrypted full-cluster generations;
- NATS file-backed JetStream stream snapshots and integrity metadata;
- model artifacts and offline signing material, with backup keys separated
  from stored ciphertext;
- infrastructure Git mirrors and immutable release manifests;
- ClickHouse schemas and rebuild inputs; ClickHouse is rebuilt from Parquet
  rather than treated as the sole truth.
- Recent compatible RocksDB checkpoints, each with a manifest containing
  block height, canonical archive manifest hash, schema versions, and state hash.
  At least one recent compatible checkpoint is replicated independently to the
  secondary operator-controlled site. Retention is current plus checkpoints;
  the production-hardening policy must set a measured checkpoint interval and
  minimum recent-generation count before automation.

MinIO's archived upstream image remains on HOLD. A one-way `mc mirror` does not
preserve complete version history or metadata and is not accepted as a backup.
Future storage must use version-aware replication or immutable exported
generations.

## Required policy before automation

The production-hardening stage must fill and approve:

- named primary and backup owners plus escalation contacts;
- encrypted off-site generations and independently controlled recovery keys;
- retention by dataset and legal/security deletion constraints;
- measured RPO and RTO values (both are **TBD and unproven** in Stage 0);
- per-generation manifests, hashes, source identity, schema version, and
  immutable creation time;
- capacity monitoring, failed-job metrics, alert delivery, and tested rollback;
- quarterly clean-room drills with preserved evidence.

## Restore safety contract

Every future restore defaults to a fresh isolated target. Before any mutation,
the procedure must print the target identity, reject any production identifier,
reject a non-empty destination, refuse overwrite, and require named operator
approval. Stage 0 examples never use force flags, mirror deletion, recursive
deletion, broad paths, or in-place production restore.

Acceptance is dataset-specific:

- PostgreSQL: `pg_verifybackup`, required WAL availability, isolated cluster
  startup, and known-query validation.
- NATS: snapshot integrity checking and restore into a new account and stream
  namespace where the same stream name cannot collide.
- MinIO/object archive: version-aware immutable generations, never
  `mc mirror --remove`.
- ClickHouse: rebuild from immutable Parquet followed by deterministic query
  and checkpoint evidence.
- RocksDB: restore the nearest compatible checkpoint into a new empty state
  directory, verify every manifest/file hash and the recorded state hash, then
  replay subsequent events from the referenced canonical archive manifest.
  Acceptance requires the reconstructed deterministic state hash to equal the
  known checkpoint and a second independently computed post-replay hash.

Recovery is successful only after all manifests and integrity checks pass and
the repository's deterministic state hashes equal known checkpoints.

## Explicitly unproven gates

- Ubuntu 24.04 VM storage ownership and mount behavior.
- Encryption, key recovery, off-site access, and credential rotation.
- PostgreSQL PITR and WAL completeness.
- NATS snapshot and namespace restore.
- Version-aware object recovery.
- ClickHouse reconstruction from Parquet.
- RocksDB checkpoint replication recency, compatible restore, subsequent-event
  replay, and deterministic state-hash equality.
- Corruption detection, measured RPO/RTO, and quarterly drill execution.
- End-to-end backup failure metrics and operator alert delivery.
