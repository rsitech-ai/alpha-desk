# Dependency and supply-chain policy

This policy applies to the complete Alpha Desk Rust workspace, including
normal, build, and development dependencies. Stage 0 quality is deterministic,
locked, offline, and pinned to Rust 1.97.1. A separately reported online
advisory refresh is freshness evidence; it is not a hidden network dependency
of local quality.

## Required local gates

Run:

```sh
cargo +1.97.1 metadata --format-version 1 --locked --offline
cargo +1.97.1 test -p architecture-check --locked --offline
cargo +1.97.1 run -p architecture-check --locked --offline -- check
./tools/ci/check-unsafe.sh
cargo +1.97.1 deny --version # must print exactly: cargo-deny 0.20.2
cargo +1.97.1 deny --locked --offline check bans licenses sources
cargo +1.97.1 deny --locked --offline check advisories
just quality
```

`just quality` runs formatting, clippy for all targets and features, the
architecture and unsafe gates, and all cargo-deny checks without network
access. Before evaluating policy, the recipe fails closed unless the executable
reports exactly `cargo-deny 0.20.2`. `just verify` includes `quality` plus the
workspace Rust and Swift test suites.

Refresh the RustSec evidence separately when network access is authorized:

```sh
cargo +1.97.1 deny --locked check advisories
```

The offline advisory result proves the committed lockfile against the locally
cached advisory database and its staleness limit. The online result proves
freshness at the time printed in the implementation or CI report.

## Architecture boundary

The normative direction and diagnostic behavior are in
[`ADR 0001`](../adr/0001-dependency-direction.md). The checker must consume
`resolve.nodes[].deps`, reconcile its PackageId set with
`resolve.nodes[].dependencies`, preserve PackageId identity, include dev edges,
and fail closed on incomplete or internally inconsistent metadata. Dependency
names and duplicate renamed aliases do not affect that set comparison. Adding
a new layer, exception, or V1 execution package requires a focused failing
fixture before the policy is changed.

## License policy

The global allowlist contains only these exact SPDX identifiers or expressions:

- `Apache-2.0`
- `Apache-2.0 WITH LLVM-exception`
- `MIT`
- `BSD-2-Clause`
- `BSD-3-Clause`
- `ISC`
- `Unicode-3.0`
- `Unicode-DFS-2016`
- `CDLA-Permissive-1.0`
- `CDLA-Permissive-2.0`

Zlib is not globally allowed. The only exception is `foldhash@0.1.5`, reached
through the build-only
`api-contracts -> tonic-prost-build -> prost-build -> petgraph -> hashbrown`
path. Remove the exception when that exact path stops resolving the package.
Any other Zlib package fails the license check.

License exceptions must name an exact package version, the otherwise rejected
license, the dependency path, and a concrete removal condition. New global
license identifiers require policy-owner review.

## Duplicate versions and dependency pins

`multiple-versions = "deny"` and
`multiple-versions-include-dev = true`. `skip-tree` is empty. The six current
transition exceptions are exact:

| Exact skip | Verified transition path and removal trigger |
|---|---|
| `getrandom@0.4.3` | `tempfile 3.27.0` and `prost-build 0.14.4` use 0.4 while `rand_core 0.9.5` uses 0.3; remove after those consumers converge. |
| `hashbrown@0.15.5` | `prost-build 0.14.4 -> petgraph 0.8.3` uses 0.15 while `indexmap 2.14.0` uses 0.17; remove after petgraph converges. |
| `r-efi@6.0.0` | follows `getrandom 0.4.3`, while `getrandom 0.3.4` uses `r-efi 5.3.0`; remove with the getrandom transition. |
| `syn@2.0.119` | prost/futures/tokio/tracing/wasm macro paths remain on syn 2 while serde/thiserror 2/async-trait use syn 3; remove after macro consumers converge. |
| `thiserror@1.0.69` | `prometheus 0.14.0 -> protobuf 3.7.2` retains thiserror 1 while workspace APIs use 2; remove after protobuf converges. |
| `thiserror-impl@1.0.69` | proc-macro companion of the same protobuf transition; remove together with `thiserror@1.0.69`. |

All local path dependencies include exact `version = "=0.1.0"` requirements.
Registry dependencies use the workspace dependency table and the committed
lockfile. Wildcard requirements fail cargo-deny. A transition exception may
skip one exact package version only and must not hide a transitive subtree.

## Registries and Git sources

The only approved registry is crates.io at
`https://github.com/rust-lang/crates.io-index`. Unknown registries and all Git
sources fail. The Stage 0 Git allowlist is empty.

A future Git dependency needs a reviewed repository URL and a Cargo `rev`
containing exactly 40 lowercase hexadecimal characters. Review must compare
that revision with the resolved full commit in `Cargo.lock`; short hashes,
branches, tags, pull-request refs, and other mutable names are rejected.
`required-git-spec = "rev"` is necessary but not sufficient, so a future
non-empty Git allowlist must ship an independent full-40-hex validator and
focused rejection tests in the same change.

## Unsafe Rust

The authoritative Stage 0 gate is:

```sh
RUSTFLAGS='-Funsafe_code' \
  cargo +1.97.1 check --workspace --all-targets --all-features --locked --offline
```

`tools/ci/check-unsafe.sh` runs that compiler gate after validating
`tools/ci/unsafe-allowlist.toml`. It clears ambient compiler, wrapper, encoded
flags, build flags, and target-specific Rust flags before setting the forbid
lint. `forbid(unsafe_code)` cannot be lowered by crate-level
`#![allow(unsafe_code)]`, and caller configuration cannot cap or allow the lint.
The Stage 0 contract is exactly schema version 1 with an empty waiver list. Any
non-empty list fails before Cargo runs. Raw grep or regular-expression scanning
is not an unsafe-code authority because it cannot distinguish Rust syntax from
comments, strings, generated output, or conditional code.

Residual boundary: the compiler gate covers the host and targets actually
selected on the configured builder. It does not prove target-specific code
behind `cfg` branches for targets that are neither installed nor built.

A later waiver design must not weaken the compiler gate silently. It requires a
syntax-aware deterministic implementation and tests for real unsafe syntax
versus comments and strings. Each waiver must use a repo-relative POSIX path,
`sha256:<64 lowercase hex>` of the exact normalized UTF-8 source line, a
non-empty reviewer and rationale, and a `YYYY-MM-DD` expiry. Expiry is
UTC-exclusive: `today_utc >= expiry` fails. Duplicate identical unsafe lines
must be rejected or disambiguated by an explicit ordinal/context field.

## Change procedure

1. Add or update a focused failing architecture, unsafe, license, source, or
   duplicate-policy test.
2. Capture the expected failure without broadening another policy.
3. Make the smallest pinned change and update `Cargo.lock` offline.
4. Inspect `cargo tree -i <name>@<version>` for every exception.
5. Run the required local gates and the separately labeled online advisory
   refresh.
6. Record the dependency path, owner, removal trigger, and any external
   freshness limitation in the implementation or CI report.
