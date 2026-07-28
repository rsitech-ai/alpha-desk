# Governance

Alpha Desk is stewarded by RSI Tech. The current lead maintainer is [Andrzej Zaborowski (`s1korrrr`)](https://github.com/s1korrrr).

## Decision making

- Routine fixes and scoped implementation changes are accepted through reviewed pull requests.
- Public contract, architecture, security-boundary, data-policy, dependency-policy, and stage-gate changes require an explicit design or decision record.
- License, trademark, repository ownership, release, and publication decisions remain with the project owner.
- Stage approval requires the roles and evidence defined by that stage's committed gate; maintainer approval does not substitute for independent review.

When consensus is not reached, the lead maintainer records the decision, alternatives, evidence, and rollback path. Safety, evidence integrity, and the no-execution V1 boundary take precedence over schedule.

## Maintainer responsibilities

Maintainers are expected to:

- protect private reports and contributor data;
- enforce the public/private repository boundary;
- review provenance, tests, and release claims;
- disclose conflicts of interest relevant to a decision;
- apply the [Code of Conduct](CODE_OF_CONDUCT.md);
- avoid claiming runtime, security, legal, or performance proof that was not performed.

## Becoming a maintainer

Maintainer access is granted after sustained, high-quality contributions and demonstrated judgment around deterministic correctness, security, privacy, and review. Access scope should follow least privilege and may be reduced when it is no longer needed.

## Release authority

No contributor or automation may publish packages, create releases, transfer repositories, change visibility, sign stage tags, or deploy production systems solely because local checks pass. Those actions require the explicit authority and evidence described in [docs/RELEASE.md](docs/RELEASE.md).
