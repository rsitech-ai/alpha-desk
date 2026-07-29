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
./tools/ci/check-dependency-exceptions.sh
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
and fail closed on incomplete or internally inconsistent metadata. A valid
workspace has at least one member, and every reachable package has exactly one
resolve node. Dependency names and duplicate renamed aliases do not affect the
dependency-set comparison. Adding a new layer, exception, or V1 execution
package requires a focused failing fixture before the policy is changed.

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

Zlib is not globally allowed. The exact exceptions are `foldhash@0.1.5` on the
build-only
`api-contracts -> tonic-prost-build -> prost-build -> petgraph -> hashbrown`
path, `foldhash@0.2.0` on the
`stage-gate -> jsonschema -> referencing -> hashbrown` path, and
`zlib-rs@0.6.6` on the `DataFusion -> Parquet -> flate2` path. Remove each
exception when its exact path stops resolving the package. Any other Zlib
package fails the license check.

CC0-1.0 is not globally allowed. The only exception is
`tiny-keccak@2.0.2`, reached by the all-target
`Arrow/Parquet -> ahash compile-time-rng -> const-random` path. CC0-1.0 is
free/libre, but the exception remains exact so a new CC0 dependency requires
review. Remove it when the pinned Arrow/Parquet line no longer resolves that
package.

License exceptions must name an exact package version, the otherwise rejected
license, the dependency path, and a concrete removal condition. New global
license identifiers require policy-owner review.

## Advisory exceptions

`RUSTSEC-2024-0436` is ignored only because the upstream Apache Parquet 58.4.0
line still resolves `paste@1.0.15`, the advisory reports unmaintained status
rather than a vulnerability, and no safe Parquet upgrade removes it. The
archive implementation does not invoke `paste` directly. The committed
lockfile and inverse dependency evidence bind the reviewed package and path.
Remove the ignore immediately when the pinned Parquet/DataFusion-compatible
line stops resolving `paste`; any vulnerability advisory remains a release
blocker. The executable exception check accepts only Parquet 58.4.0, the exact
DataFusion 54.1.0 crate family, the reusable `canonical-archive` foundation,
and its reviewed `hl-capture`, `hl-analytics`, and `archive-inspect` consumers
in the inverse path. `spool-inspect` is also an allowed inverse root because it
uses the `hl-capture` library's spool contracts; it does not invoke the archive.
An unrelated consumer or version fails closed.

## Duplicate versions and dependency pins

`multiple-versions = "deny"` and
`multiple-versions-include-dev = true`. `skip-tree` is empty. The current
transition exceptions are exact:

| Exact skip | Verified transition path and removal trigger |
|---|---|
| `getrandom@0.2.17` | Arrow/Parquet all-target validation enables `ahash` compile-time RNG through `const-random`, while native `ahash` and workspace randomness use 0.3; remove when the all-target feature graph converges. |
| `getrandom@0.4.3` | `tempfile 3.27.0` and `prost-build 0.14.4` use 0.4 while `rand_core 0.9.5` uses 0.3; remove after those consumers converge. |
| `foldhash@0.2.0` | `jsonschema 0.48.5 -> referencing 0.48.5` uses 0.2 while the prost build path retains 0.1; remove after those consumers converge. |
| `hashbrown@0.14.5` | `DataFusion 54.1.0 -> dashmap 6.2.1` uses 0.14 while Arrow 58.4.0 and `indexmap` use 0.17; remove after DashMap converges. |
| `hashbrown@0.15.5` | `prost-build 0.14.4 -> petgraph 0.8.3` uses 0.15 while `indexmap 2.14.0` uses 0.17; remove after petgraph converges. |
| `phf@0.13.1` | `tokio-postgres 0.7.18` uses PHF 0.13 for PostgreSQL type metadata while the pinned Arrow time-zone graph retains PHF 0.12; remove when those owners converge. |
| `phf_shared@0.13.1` | companion of the `tokio-postgres -> phf 0.13` transition while the Arrow time-zone graph retains 0.12; remove with the PHF transition. |
| `r-efi@6.0.0` | follows `getrandom 0.4.3`, while `getrandom 0.3.4` uses `r-efi 5.3.0`; remove with the getrandom transition. |
| `syn@2.0.119` | prost/futures/tokio/tracing/wasm macro paths remain on syn 2 while serde/thiserror 2/async-trait use syn 3; remove after macro consumers converge. |
| `thiserror@1.0.69` | `prometheus 0.14.0 -> protobuf 3.7.2` retains thiserror 1 while workspace APIs use 2; remove after protobuf converges. |
| `thiserror-impl@1.0.69` | proc-macro companion of the same protobuf transition; remove together with `thiserror@1.0.69`. |
| `wasi@0.14.7+wasi-0.2.4` | `tokio-postgres 0.7.18 -> whoami 2.1.2 -> wasite 1.0.2` supports WASI 0.2 while `getrandom` retains the legacy WASI 0.11 target package; remove when those target-support paths converge. |

The pinned `async-nats 0.50.0` client enables only JetStream, NKey
authentication, Ring TLS, and NATS Server 2.14 compatibility. Alpha Desk does
not enable its KV, object-store, service, NUID, WebSocket, or experimental
features. Its current transport and `nkeys 0.4.5` authentication graph adds the
following exact transitions:

| Exact skip | Verified transition path and removal trigger |
|---|---|
| `block-buffer@0.10.4` | NKey signing uses the RustCrypto digest 0.10 line while workspace archive hashing uses block-buffer 0.12; remove when nkeys converges. |
| `const-oid@0.9.6` | `nkeys -> pkcs8/der` retains const-oid 0.9 while SHA-2 0.11 uses 0.10; remove when the signing graph converges. |
| `cpufeatures@0.2.17` | NKey signing retains RustCrypto cpufeatures 0.2 while workspace SHA-2/BLAKE3 uses 0.3; remove when the signing graph converges. |
| `crypto-common@0.1.7` | NKey digest 0.10 retains crypto-common 0.1 while workspace digest 0.11 uses 0.2; remove when the signing graph converges. |
| `digest@0.10.7` | NKey signing retains digest 0.10 while workspace hashing uses 0.11; remove when nkeys converges. |
| `rand@0.8.7` | `nkeys 0.4.5` uses rand 0.8 while analytics uses 0.9 and async-nats internals use 0.10; remove when nkeys converges. |
| `rand@0.10.2` | async-nats transport internals use rand 0.10 while the analytics graph uses 0.9; remove when those owners converge. |
| `rand_chacha@0.3.1` | NKey signing retains rand_chacha 0.3 while analytics uses 0.9; remove when nkeys converges. |
| `rand_core@0.6.4` | NKey signing retains rand_core 0.6 while analytics/transport use 0.9/0.10; remove when nkeys converges. |
| `rand_core@0.10.1` | async-nats transport uses rand_core 0.10 while analytics uses 0.9; remove when those owners converge. |
| `sha2@0.10.9` | NKey signing retains SHA-2 0.10 while archive/publication hashing uses 0.11; remove when nkeys converges. |
| `windows-sys@0.52.0` | async-nats native certificate discovery reaches schannel/windows-sys 0.52 while current Tokio/filesystem paths use 0.61; remove when rustls-native-certs converges. |

`tools/ci/check-dependency-exceptions.sh` resolves every listed inverse tree
and fails if any transport exception is no longer owned by
`async-nats 0.50.0` or `nkeys 0.4.5`. These exact entries are not a general
allowance for duplicate cryptography or randomness libraries.

The same executable check binds the PHF and WASI transition entries to the
exact `tokio-postgres 0.7.18` graph. The progress journal uses
`tokio-postgres` without its optional Chrono, JSON, UUID, Jiff, time, or
JavaScript features; block heights, stream sequences, and cursor versions are
encoded as validated decimal text so PostgreSQL `numeric` preserves the full
unsigned 64-bit domain.

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
`tools/ci/unsafe-allowlist.toml`. It resolves and validates the Rust 1.97.1
Cargo and compiler, then overrides both direct and `CARGO_BUILD_*` compiler and
wrapper settings. Empty wrapper values explicitly reset Cargo configuration
wrappers, as specified by the
[Cargo environment-variable contract](https://doc.rust-lang.org/cargo/reference/environment-variables.html).
The gate uses a temporary Cargo home containing only linked registry index and
packaged-crate cache inputs; configuration, credentials, extracted sources, and
artifacts are excluded. Every run also uses fresh private target and build
directories removed on exit. Ambient compiler flags, encoded flags, and
target-specific Rust flags are cleared before setting the forbid lint.
`forbid(unsafe_code)` cannot be lowered by crate-level
`#![allow(unsafe_code)]`, and caller configuration or cached artifacts cannot
cap or allow the lint. The Stage 0 contract is exactly schema version 1 with an
empty waiver list. Any non-empty list fails before Cargo runs. Raw grep or
regular-expression scanning is not an unsafe-code authority because it cannot
distinguish Rust syntax from comments, strings, generated output, or
conditional code.

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
