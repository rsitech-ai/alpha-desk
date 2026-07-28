# Roadmap

The canonical roadmap is the approved [program plan](superpowers/plans/2026-07-24-00-hyperliquid-alpha-desk-program-roadmap.md). This page is the contributor-facing sequence, not a replacement for the detailed gate contracts.

## Current focus

1. Keep hosted CI, trust identities, reviewer signatures, evidence commits, and signed tags explicitly blocked until their external prerequisites exist.
2. Review the implemented Stage 1 source-observation contracts and strict capture configuration.
3. Continue with the crash-safe append-only spool.
4. Add primary and independent source adapters behind the typed source ports.
5. Build continuity, quarantine, archive, replay, and publication before claiming a meaningful long-running product test.

## Required order

| Order | Outcome | Why it precedes the next stage |
| ---: | --- | --- |
| 0 | Reproducible, policy-enforced foundation | Later evidence is meaningful only when builds, schemas, provenance, and trust contracts are stable |
| 1 | Durable and contiguous truth layer | State and research must be reproducible from retained source evidence |
| 2 | Deterministic state reconstruction | Intelligence features need correct historical state |
| 3 | Wallet and entity intelligence | Market cohorts require measured, versioned attribution |
| 4 | Health-gated signals | Research must evaluate evidence-linked signals, not ad hoc indicators |
| 5 | Reproducible alpha laboratory | Product surfaces consume only reviewed research contracts |
| 6 | Internal read-only desk | UI follows stable APIs and evidence semantics |
| 7 | Production and OSS qualification | Release requires restore, load, soak, security, canary, rollback, and public-export proof |

## Near-term contributor slices

- Stage 1 spool format, recovery scanner, and corruption tests.
- Primary and independent source adapters, with proprietary operator-feed material kept outside the public repository.
- Deterministic canonicalization, continuity/quarantine, immutable archive, and replayable publication.
- Only after those foundations: a long-running capture service and meaningful restart/replay/soak evidence.

## Deliberately deferred

- UI mockups that establish contracts ahead of the canonical API.
- Trading execution, signing, custody, and order placement.
- Public release of private alpha, production models, wallet labels, or infrastructure inventory.
- Claims of signal quality or profitability before leakage-controlled out-of-sample evidence exists.
