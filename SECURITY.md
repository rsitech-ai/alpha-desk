# Security Policy

## Supported versions

Alpha Desk has not released a supported public version. The current repository is pre-release engineering work. Security fixes will target the current development line until a versioned support policy is published.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability, exposed credential, private data, proprietary feed material, or exploit details.

Use GitHub private vulnerability reporting on the canonical repository when it is enabled. If that surface is unavailable, contact RSI Tech privately at `info@rsitech.ai` with:

- affected commit or version;
- impact and preconditions;
- minimal reproduction;
- whether secrets or private data may be exposed;
- a safe way to coordinate follow-up.

Do not include live credentials, private keys, or third-party personal data. Use synthetic evidence where possible. You should receive an acknowledgement within five business days; this is a response target, not a guaranteed remediation timeline.

## Scope

Reports about source capture integrity, evidence loss, silent divergence, authentication/authorization, secret handling, unsafe deserialization, dependency compromise, path traversal, process containment, or bypass of the read-only boundary are especially valuable.

Trading loss, market movement, model quality, and unsupported deployment configurations are not security vulnerabilities by themselves. Any route that introduces signer or order-placement capability into V1 should be treated as a critical boundary violation.

## Disclosure

Please allow time to validate and remediate a report before public disclosure. Coordinated disclosure details will be agreed per report. Publication readiness also requires an independent security review; the presence of this policy is not evidence that such a review has passed.
