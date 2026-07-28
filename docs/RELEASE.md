# Release and Publication

Alpha Desk is currently in local Prepare mode. No public repository, package, application, or supported version has been released.

## Readiness levels

- `code-ready`: a scoped change passes its focused local checks.
- `runtime-proven`: the real process or product path passes the documented runtime evidence.
- `oss-ready`: the deterministic public export, history/provenance review, community surface, legal decision, and clean-room verification pass.
- `preview-ready`: the runnable public preview, onboarding, limitations, and representative acceptance evidence pass.
- `release-ready`: all signed stage gates, security, restore, load, soak, canary, rollback, artifact, SBOM, and approval requirements pass.

These labels are cumulative. Local source compilation alone does not establish any release label.

## Repository boundary

The private engineering repository must not be made public in place. The intended public target is a clean, reviewed `rsitech-ai/alpha-desk` export containing the platform surface only. Private alpha, models, wallet labels, operator-feed material, production inventory, secrets, certificates, and deployment topology remain outside it.

The public export and its history must pass the policy and audit described under `docs/open-source/`. Current recovery branches and encoded transport artifacts require explicit exclusion and history review.

## License and identity blockers

The current source license is Apache-2.0. The design's possible dual-license recommendation remains `blocked:license-decision` pending owner/legal review. Repository creation, ownership transfer, visibility, descriptions/topics, private vulnerability reporting, signing identities, and publication are `blocked:external`.

## Artifacts and checksums

Checksums belong to immutable release artifacts generated from an exact clean source commit. Do not commit checksums for mutable README or design files and do not describe prose placeholders as checksums.

A future release process must:

1. verify a clean immutable source commit and signed stage evidence;
2. build in clean, pinned environments;
3. generate SBOM and provenance;
4. produce exact artifacts;
5. write a checksum manifest from those artifact bytes;
6. verify every checksum from a second clean environment;
7. publish only after approvals and external authority are present.

## Current exit blockers

See [docs/STATUS.md](STATUS.md) for the live blocker list. Until those items close, documentation and manifests must continue to say pre-release, Prepare-only, and not runtime-proven.
