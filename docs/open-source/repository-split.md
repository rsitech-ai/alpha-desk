# Public Repository Split

The private engineering repository is not the public artifact. Alpha Desk's release design separates:

- the open platform: generic source interfaces, canonical ledger and replay framework, public contracts, generic research tooling, native client shell, synthetic fixtures, and public documentation;
- the private alpha pack: proprietary feature compositions, cohort definitions, trained production models, thresholds, promotion evidence, and capital configuration;
- the private deployment repository: inventory, certificates, secrets, operator identities, production topology, and environment-specific policy.

## Current policy

[`config/open-source-policy.toml`](../../config/open-source-policy.toml) classifies every current top-level path. `public` paths are candidates for export. `generated-review-required` paths require an explicit release review. `private` and `excluded` paths never enter the public tree.

Run the working-tree audit:

```sh
just oss-audit
```

It inventories tracked and non-ignored untracked files, fails on unclassified roots, rejects historical transport fragments, applies bounded file/binary policy, and scans for versioned secret/private-alpha/local-path/execution canaries. Its tests contain seeded canaries that must remain detectable.

## Publication history

The current private remote contains a recovery branch and draft pull request with encoded, incomplete source-transport fragments. Ordinary secret scanners do not fully inspect those blobs. The personal engineering repository must therefore remain private.

Before public preview:

1. freeze an exact reviewed implementation commit;
2. create a clean export from an allowlist, never by recursively copying the working directory;
3. exclude private, generated, ignored, local-evidence, database, model, archive, certificate, and key material;
4. review every `generated-review-required` path;
5. scan the export and a fully fetched mirror of the intended public history;
6. verify author/committer attribution and recreate only approved tags;
7. build and test from a clean clone of the exported history;
8. publish to the canonical `rsitech-ai/alpha-desk` repository only with explicit external authority.

A deterministic export writer is still required before OSS readiness. Until it exists and is tested against this policy, the repository remains Prepare-only.
