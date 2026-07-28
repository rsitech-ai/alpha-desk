# Contributing

Alpha Desk is being built through staged, evidence-backed plans. Contributions are welcome when they preserve the read-only safety boundary and fit the current stage.

## Before opening a change

1. Read [docs/STATUS.md](docs/STATUS.md) and [docs/ROADMAP.md](docs/ROADMAP.md).
2. Find the owning contract in the relevant plan under `docs/superpowers/plans/`.
3. Open an issue for changes that alter public contracts, architecture, gate criteria, dependencies, data policy, or licensing.
4. Keep private alpha, operator data, wallet labels, secrets, and deployment details outside the repository.

## Development workflow

- Branch from the reviewed baseline.
- Add a focused failing test before behavior changes.
- Keep commits scoped and explain the invariant or user outcome.
- Run the strongest relevant local checks:

```sh
just verify
just generated
```

For infrastructure changes, also run:

```sh
just stage-0-compose-smoke
```

Hosted CI evidence, signed gate records, and runtime qualification are separate from local success.

## Pull requests

A pull request should include:

- the problem and the intended outcome;
- the affected design or plan requirement;
- tests added or changed;
- exact verification commands and results;
- security, privacy, data, compatibility, and rollback impact;
- an explicit statement when runtime proof was not performed.

Do not mix unrelated cleanup with behavior changes. Generated files must be reproducible from committed inputs.

## Review expectations

Reviewers check correctness, deterministic behavior, bounded resources, failure semantics, provenance, documentation truth, and compatibility with stage gates. A green build alone is not sufficient evidence for a stage or release claim.

By participating, you agree to follow [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Security vulnerabilities belong in the private process described by [SECURITY.md](SECURITY.md).
