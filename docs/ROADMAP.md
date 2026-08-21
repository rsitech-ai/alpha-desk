# Roadmap

The V1 program plan file `docs/superpowers/plans/2026-07-24-00-hyperliquid-alpha-desk-program-roadmap.md` is absent on this branch. Do not invent it. Frozen V1 tags remain `design-approved-v1.0.0` and `spec-v1.0.0`. The V1 design file `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md` is also absent here. This page is the contributor-facing sequence, not a replacement for those gate contracts.

## Follow-on: Hyperliquid full public-data coverage

Additive to the 2026-07-24 design. It does not replace V1 and may proceed in parallel with remaining V1 work on this mainline tree.

- Spec: [superpowers/specs/2026-08-19-hyperliquid-full-coverage-expansion.md](superpowers/specs/2026-08-19-hyperliquid-full-coverage-expansion.md)
- Plan: [superpowers/plans/2026-08-19-hyperliquid-full-coverage-plan.md](superpowers/plans/2026-08-19-hyperliquid-full-coverage-plan.md)
- Traceability: [superpowers/plans/2026-08-19-hyperliquid-full-coverage-traceability.md](superpowers/plans/2026-08-19-hyperliquid-full-coverage-traceability.md)
- Docs check: `just hyperliquid-full-coverage-docs`

## Current focus

1. Keep hosted CI, trust identities, reviewer signatures, evidence commits, and signed tags explicitly blocked until their external prerequisites exist.
2. Qualify the implemented primary-node adapters against non-secret operator node recordings.
3. Bind independent and recovery transports to the implemented fail-closed
   source-trust policy without exposing proprietary operator-feed material.
4. Add independent gap recovery and qualified action-bearing mapping before a
   meaningful long-running product test; downstream-outage spool draining is
   implemented and synthetic fault-proven.

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

- Qualify primary-node file/stream adapters with redistribution-reviewed byte-exact recordings from
  the deployed node version; checked fixtures currently remain normalized official examples.
- Independent and recovery source transports behind the implemented
  trust/admission boundary, with proprietary operator-feed material kept
  outside the public repository.
- Complete qualified source-to-canonical mappings and upcasters on the
  implemented stable event/block identity boundary.
- Independent gap recovery, canonical state replay, and correction handling;
  durable backlog draining now has bounded runtime and outage-recovery
  evidence.
- Extend the runnable capture service from short synthetic restart/soak
  evidence to multi-hour, crash-failpoint, host-restart, and real-source
  qualification.

## Deliberately deferred

- UI mockups that establish contracts ahead of the canonical API.
- Trading execution, signing, custody, and order placement.
- Public release of private alpha, production models, wallet labels, or infrastructure inventory.
- Claims of signal quality or profitability before leakage-controlled out-of-sample evidence exists.
