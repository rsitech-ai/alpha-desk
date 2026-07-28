# Public API and Plugin Boundaries

The public platform may expose versioned interfaces for:

- committed, historical, snapshot, provisional, and public-data source adapters;
- archive storage;
- deterministic feature calculators;
- signal evaluators;
- model runtimes;
- notification sinks;
- evidence panels in client applications.

Extensions cannot:

- bypass source-observation validation, durable capture, continuity, or quarantine;
- mutate canonical state from provisional observations;
- publish canonical data before archive durability;
- suppress health or provenance failures;
- access private alpha or production deployment material through a public interface;
- introduce execution, signing, custody, private-key handling, or order placement into V1.

The first public capture boundary is defined by Stage 1 Task 1 in [`2026-07-24-02-truth-layer.md`](../superpowers/plans/2026-07-24-02-truth-layer.md). It is not stable until its implementation, compatibility tests, and stage gate pass.
