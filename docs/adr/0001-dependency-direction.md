# ADR 0001: Enforce dependency direction from resolved Cargo identity

- Status: Accepted
- Date: 2026-07-27
- Scope: Alpha Desk V1 Rust workspace

## Context

Alpha Desk keeps deterministic domain behavior reusable across capture, live
analytics, historical replay, research, and the local desktop client. A
manifest review is insufficient to preserve that boundary: dependency aliases
can hide package names, dev-dependencies can form cycles Cargo permits, and
transitive edges can reach a forbidden runtime without a direct manifest edge.
V1 is strictly read-only and must not contain an `hl-exec` dependency.

## Decision

`architecture-check` obtains complete Cargo metadata with `--locked --offline`.
It indexes packages and workspace membership by the opaque
`cargo_metadata::PackageId` from pinned `cargo_metadata = 0.23.1`. Edges come
from `resolve.nodes[].deps[].pkg`, never the dependency alias in
`NodeDep.name`. Normal, build, and dev edges all participate.

Workspace packages are classified from their repo-relative manifest location:

- `crates/`: domain and domain-support crates;
- `services/`: deployable orchestration packages;
- `tools/`: development and quality tools;
- other locations: unclassified and unable to acquire an implicit exception.

The checker fails on:

- any path from `domain-types` to `storage-ports`;
- any path from `feature-core` to `model-runtime`;
- any path from a package under `crates/` to a service package;
- any workspace dependency path that reaches a package named `hl-exec`;
- any workspace cycle, including a cycle containing a dev edge;
- missing `resolve` data, unknown workspace members, unknown resolve nodes,
  unknown resolved package IDs, or duplicate metadata identities.

For each rule the diagnostic is the deterministic shortest path. Neighbors are
ordered by package name and then opaque PackageId representation. Cycle
selection is shortest first and lexicographically stable second. Package names
are presentation only; graph identity remains PackageId.

The domain owns ports; storage, network, vendor SDK, and service packages may
implement or orchestrate them. Feature definitions remain runtime-independent.
Service binaries may depend inward on domain crates, but domain crates may not
depend outward on service binaries.

## Consequences

- Renaming a dependency cannot evade architecture policy.
- Test-only edges remain subject to strict acyclicity even where Cargo can
  compile them.
- Quality checks need the complete locked dependency graph in the local cache.
- New package locations or dependency-direction exceptions require an ADR and
  a checker test before implementation.
- `hl-exec` cannot enter V1 transitively or through a renamed dependency.

## Alternatives rejected

- Manifest text scanning: it does not resolve aliases, target selection, or
  transitive identity.
- `cargo metadata --no-deps`: its missing resolve graph prevents fail-closed
  path checks.
- Ignoring dev edges: Cargo-valid dev cycles still violate the workspace's
  deterministic architecture policy.
- Comparing package names as graph keys: different package versions can share
  a name, while PackageId is the Cargo-defined resolved identity.
