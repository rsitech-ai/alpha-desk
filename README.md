# Hyperliquid Alpha Desk — Approved Design and Implementation Plans

This repository contains the approved production design and the complete staged implementation plan for a private, local-only Hyperliquid market-intelligence and alpha-research desk.

The design defines the production architecture, canonical data model, deterministic state reconstruction, wallet/entity intelligence, market-sentiment framework, signal validation, native SwiftUI desk, security boundaries, operations, testing, and phased acceptance gates. The implementation plan translates that design into reviewer-sized, test-driven tasks with exact files, interfaces, commands, expected results, commits, and stage gates.

## Canonical documents

- [Approved production design](docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md)
- [Implementation-plan index](docs/superpowers/plans/README.md)
- [Program roadmap](docs/superpowers/plans/2026-07-24-00-hyperliquid-alpha-desk-program-roadmap.md)
- [Specification traceability](docs/superpowers/plans/2026-07-24-99-spec-traceability.md)
- [Plan self-review](docs/superpowers/plans/2026-07-24-98-plan-self-review.md)

## Selected baseline

- Rust 1.97.1, edition 2024, for the canonical event-sourced core, replay, research, APIs, and tooling.
- Swift 6.3, SwiftUI, Swift Charts, GRDB, and Core ML for the native Apple desk and local personalization.
- NATS JetStream, RocksDB, ClickHouse LTS, PostgreSQL, Arrow/Parquet, DataFusion, Polars, and ONNX Runtime.
- Kanidm for self-hosted OIDC/WebAuthn.
- Dedicated Ubuntu 24.04/systemd hot path with Ansible/Podman for reproducible local deployment; no mandatory Kubernetes.
- Read-only V1 with no trading signer, exchange private key, or order-placement path.

## Status

Design version 1.0.0 was approved for implementation on 2026-07-24. The implementation-plan set is complete. The Stage 0 workspace bootstrap is in place; production domain behavior proceeds only through evidence-based stage gates.

The future execution enclave is outside V1 and requires a separate threat model, approved design, and implementation plan after shadow-live and paper evidence satisfy the admission policy.

## Stage 0 gate

The committed Stage 0 contract is
[`config/stage-gates/stage-0.toml`](config/stage-gates/stage-0.toml). Run it
only from the clean, frozen implementation commit:

```sh
just stage-0-gate
```

The command writes transient canonical JSON only to the Git-ignored
`target/stage-gates/stage-0.json` and writes the exact canonical local builder
evidence to `target/stage-gates/stage-0.builder.json`. Copy Builder B's
canonical `stage-0.builder.json` and its detached OpenPGP signature byte-for-byte
to Builder A's configured input paths; no JSON extraction or rewriting is
permitted. Builder B's report identity must be
`builder-b:<full-fingerprint>`, must agree with the pinned `builder-b` signer,
and must be distinct from Builder A and both reviewers. Exit status `0` means `PASS`, `1` means a local
verification `FAIL`, and `2` means `BLOCKED`. A local builder remains
`BLOCKED` until a signed second-builder report, the signed exact GitHub run
proof, two distinct detached reviewer approvals, a four-role trust registry,
and usable OpenPGP verification tooling are supplied. Any non-PASS
result has the explicit stage outcome `HOLD`. External reports, proofs,
signatures, and the keyring stay under the ignored input paths named by the
configuration. The gate never creates an approval record, signature, evidence
commit, or tag.

The tracked operational trust registry is
[`stage-0-trust-policy.toml`](config/stage-gates/stage-0-trust-policy.toml).
Its current placeholder fingerprints intentionally keep Stage 0 blocked. They
must be replaced by four distinct, reviewed, full fingerprints for
`platform-data`, `independent`, `builder-b`, and `github-ci` in a committed
change; the gate hashes the exact committed registry bytes. The separate
[`stage-0-trust-policy.example.toml`](config/stage-gates/stage-0-trust-policy.example.toml)
remains a non-operational template for all four roles.

### Evidence normalization

Builder comparison excludes exactly `builder_identity`, the envelope containing
the builder ID, signer metadata, and resolved executable paths. It still binds
normalized hostname-free OS identity (`uname -s -r -m`), tool IDs, executable
SHA-256 values, version output, artifact metadata and bytes, check results, and
`check_evidence_hashes`. Each check-evidence hash covers the check ID, resolved
executable hash, and exit code. Raw stdout/stderr are bounded local diagnostics
and are not published as reproducibility evidence.

### GitHub proof defaults and migration

The least-privilege `Stage 0 evidence` workflow runs only after a successful
trusted `push` CI run on `main`; its token has `actions: read`, `checks: read`,
and `contents: read`. It signs a canonical proof for the six jobs in that exact
CI run and records the separate in-progress signing job identity. It uploads
the proof, detached signature, and public key for 30 days. The job fails closed
when the dedicated `STAGE0_GITHUB_CI_PRIVATE_KEY` secret is absent.

The reviewed workflow is pinned by `remote.workflow_sha` to commit
`9996166da6f38df467fa4fc479ab80edcd5bb28f`. Before it can produce consumable
evidence:

1. Replace all four trust-policy placeholders, provision the dedicated CI
   private key, and distribute the matching public key through the reviewed
   keyring.

The current OpenPGP mechanism proves possession of a long-lived dedicated key;
it does not prove GitHub workload identity. Key custody, rotation, and
revocation remain operator responsibilities. GitHub documents that
`workflow_run` jobs can access secrets, so the workflow rejects untrusted event,
branch, and repository identities before touching the key. The future migration
target is GitHub/Sigstore artifact attestation, but GitHub currently requires
Enterprise Cloud for private-repository attestations. See GitHub's official
[workflow-run security note](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#workflow_run),
[check-runs API](https://docs.github.com/en/rest/checks/runs#list-check-runs-for-a-git-reference),
[least-privilege token guidance](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token),
and [private-repository attestation requirement](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations).

### Runtime-proof boundary

The gate uses a retained directory descriptor for evidence publication and
isolated Compose project names/resources. Static tests prove the command and
cleanup contract without touching a real Docker daemon. A real merged Compose
render and startup/cleanup smoke remain required runtime proof. Unix
process-group cleanup also cannot contain a hostile descendant that escapes
with `setsid`; such commands require stronger OS-level containment.

## V1 safety boundary

The current V1 is read-only. It contains no execution service, trading signer, exchange private-key handling, order-placement path, or signing capability. Any future execution enclave is explicitly outside this workspace boundary until separately designed, reviewed, and approved.
