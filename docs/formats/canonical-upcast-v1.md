# Canonical Upcast V1

The canonical reader has one fail-closed semantic-version boundary before
historical persisted schemas exist.

`CanonicalUpcaster::v1()` supports canonical schema `1.0.x`:

1. decode the Protobuf envelope enough to inspect `schema_version`;
2. require numeric SemVer with no pre-release or build metadata;
3. reject every major other than `1` and every minor other than `0`;
4. validate the complete envelope, payload kind, typed payload, payload hash,
   source evidence, lifecycle ordering, and current V1 invariants;
5. return the exact original input bytes without re-encoding.

Patch versions are accepted because they cannot introduce incompatible
semantics. A future minor or major requires an explicit registered migration.
Historical major `0` is unsupported because this repository has no persisted
or published V0 contract to upcast.

The V1 identity path is deliberately byte-preserving. It does not rewrite raw
source evidence, canonical payloads, unknown wire encodings, or observation
metadata. A future upcaster must be a pure explicit step from one immutable
version contract to the next; it must never mutate archived raw evidence.

Stable errors distinguish:

- `canonical_upcast.malformed_envelope`;
- `canonical_upcast.malformed_version`;
- `canonical_upcast.unsupported_version`; and
- `canonical_upcast.invalid_current_envelope`.

Adding a supported minor/major range without a concrete migration and golden
fixtures is a compatibility violation.
