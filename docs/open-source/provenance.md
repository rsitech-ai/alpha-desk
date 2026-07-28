# Source and Release Provenance

## Generated material

- Protobuf descriptor baseline: `schemas/proto/baseline/v1.pb`
- Rust contract output and schema fingerprints: regenerated and checked by `just generated`
- Service build identity: produced by the `build-info` and telemetry provenance tooling

Generated files must be reproducible from committed inputs. Review generated binary changes through their generator and compatibility checks rather than raw diff alone.

## Source identity

Local verification records the exact Git commit and dirty state. A release requires a clean immutable commit, approved author/committer attribution, signed or attested release identity, and provenance tied to exact artifact bytes.

The current design/spec tags are approval metadata but are not cryptographically signed release evidence.

## Dependencies and containers

Rust dependencies are locked and checked by `cargo-deny`. Container images are pinned in `infra/docker-compose/images.lock`. Release work must produce an SBOM and third-party notice inventory from exact lockfiles and must distinguish development services from bundled distributable components.

## Release artifacts

Release checksums are generated only after immutable artifacts exist. Each checksum manifest must use exact SHA-256 values and paths, reject duplicates and missing files, and verify in a second clean environment.
