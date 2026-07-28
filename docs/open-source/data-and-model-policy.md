# Data and Model Policy

## Public data

The public repository may contain only:

- synthetic fixtures created for deterministic tests;
- redacted fixtures whose provenance and redistribution rights are documented;
- public protocol/schema examples with a recorded source and license;
- generated compatibility artifacts reproducible from committed public inputs.

It must not contain real wallet labels, private operator-feed samples, customer data, private historical corpora, production databases, or source evidence collected under a restricted agreement.

## Private alpha and models

Feature compositions, cohort definitions, thresholds, trained production models, promotion results, capital configuration, and live inference evidence belong in the private alpha pack. Public interfaces may describe generic feature or model-runtime contracts without shipping proprietary weights or decisions.

## Research claims

Every published research result must identify:

- data and observation window;
- cost, fee, funding, spread, slippage, latency, and leakage assumptions;
- train/validation/test separation;
- denominator and uncertainty;
- regime and capacity limits;
- reproducible evidence and version identity.

No output is guaranteed trading advice or a promise of profitability.

## Fixture provenance

`fixtures/golden/` is synthetic and intended for deterministic contract tests. New fixtures require a provenance note, redistribution decision, and a check that no real identity or private data was copied into the repository.
