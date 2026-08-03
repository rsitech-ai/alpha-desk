# Node Source Qualification V1

Status: implemented contract, empty production registry.

This contract separates a byte-recording claim from authority to produce
committed joined trades. Decoding a manifest proves only that its JSON is
bounded, canonical, internally consistent, and equal to a caller-supplied
SHA-256. It does not prove that the node binary, command line, recording, or
parser claims are true.

The only authority type is `QualifiedNodeSourceV1`. Its fields are private and
the production constructor compares the canonical manifest SHA-256 against an
internal compiled registry. That registry is intentionally empty until the M4
operator-corpus gate is complete. There is no configuration boolean, source
version string, fixture label, public registry constructor, or deserializer
that can create the token.

## Canonical manifest

Schema identifier:

```text
hyperliquid-alpha-desk/node-source-qualification/v1
```

The UTF-8 JSON document must be at most 1 MiB, contain no unknown fields, and
equal its compact deterministic re-encoding byte-for-byte. Leading/trailing
whitespace, alternate field order, duplicate/unknown fields, uppercase digest
hex, and noncanonical escaping fail closed. The expected SHA-256 is verified
before JSON parsing or registry lookup.

The manifest binds:

- recording, chain, and stable physical node-instance identities;
- distinct committed-block and trade-output source IDs;
- node artifact name/version/repository commit, exact build argv, binary and
  build-material SHA-256, and signature fingerprint/evidence material;
- exact runtime argv and runtime-material SHA-256;
- explicit output-file buffering mode;
- committed/trade parser, mapper, market-catalog, and time-normalization rule
  versions plus independent material SHA-256 hashes;
- redistribution classification;
- every retained raw file's safe relative path, source role, rotation sequence,
  nonzero size, SHA-256, and first/last native cursor evidence;
- native cursor epoch, role-typed position, and BLAKE3 content digest. Committed
  files use block height; trade line files use the exclusive byte end-offset
  emitted after a complete line. Local spool sequence is deliberately absent.

SHA-256 and BLAKE3 are different Rust types and different JSON field names.
Both require exactly 64 lowercase hexadecimal characters.

M1 intentionally uses raw JSON as the generation boundary: it exposes complete
read-only getters and a strict decoder, but no public builder or authority
constructor. The following multiline template documents the complete field
order and shape; it is not canonical as printed. Replace every placeholder,
then encode the same order as one compact JSON value with no trailing newline.

```json
{
  "schema": "hyperliquid-alpha-desk/node-source-qualification/v1",
  "recording_id": "<identity>",
  "chain_id": "<identity>",
  "node_instance_id": "<identity>",
  "source_group": {
    "committed_source_id": "<identity>",
    "trade_source_id": "<different-identity>"
  },
  "artifact": {
    "name": "hyperliquid-node",
    "version": "<identity>",
    "repository_commit": "<40-or-64-lowercase-hex>",
    "build_argv": ["<argument>"],
    "binary_sha256": "<64-lowercase-hex>",
    "build_material_sha256": "<64-lowercase-hex>",
    "signature_fingerprint": "<identity>",
    "signature_material_sha256": "<64-lowercase-hex>"
  },
  "capture": {
    "argv": ["<executable-and-arguments>"],
    "output_file_buffering": "disabled",
    "production_recording": true,
    "same_node_instance": true,
    "byte_exact": true,
    "corpus_coverage_complete": true,
    "runtime_material_sha256": "<64-lowercase-hex>"
  },
  "profile": {
    "qualification_profile": "<identity>",
    "committed_parser_version": "<identity>",
    "committed_parser_material_sha256": "<64-lowercase-hex>",
    "trade_parser_version": "<identity>",
    "trade_parser_material_sha256": "<64-lowercase-hex>",
    "mapper_version": "<identity>",
    "mapper_material_sha256": "<64-lowercase-hex>",
    "catalog_version": "<identity>",
    "catalog_sha256": "<64-lowercase-hex>",
    "time_normalization_rule": "<identity>",
    "time_normalization_material_sha256": "<64-lowercase-hex>"
  },
  "redistribution": "private-operator-evidence",
  "files": [
    {
      "relative_path": "replica_cmds/000000.ndjson",
      "role": "committed",
      "rotation_sequence": 0,
      "size_bytes": 1,
      "sha256": "<64-lowercase-hex>",
      "first_cursor": {
        "epoch": "<identity>",
        "position": {"kind": "block-height", "height": 1},
        "content_blake3": "<64-lowercase-hex>"
      },
      "last_cursor": {
        "epoch": "<same-identity>",
        "position": {"kind": "block-height", "height": 1},
        "content_blake3": "<64-lowercase-hex>"
      }
    },
    {
      "relative_path": "block_trades/000000.ndjson",
      "role": "trade",
      "rotation_sequence": 0,
      "size_bytes": 1,
      "sha256": "<64-lowercase-hex>",
      "first_cursor": {
        "epoch": "<identity>",
        "position": {"kind": "byte-offset", "end_offset": 1},
        "content_blake3": "<64-lowercase-hex>"
      },
      "last_cursor": {
        "epoch": "<same-identity>",
        "position": {"kind": "byte-offset", "end_offset": 1},
        "content_blake3": "<64-lowercase-hex>"
      }
    }
  ]
}
```

Enforced bounds:

- manifest: 1 MiB before JSON deserialization;
- identity text: 1–256 UTF-8 bytes, trimmed, without control characters;
- repository commit: exactly 40 or 64 lowercase hexadecimal characters;
- build/runtime argv: 1–128 entries, each 1–4096 UTF-8 bytes, trimmed, without
  control characters;
- recording files: 1–4096 descriptors, with both roles represented;
- relative path: 1–4096 UTF-8 bytes plus the component restrictions below.

## Runtime argv contract

V1 requires exactly one occurrence of each documented source-output flag:

```text
--write-trades
--batch-by-block
--replica-cmds-style actions-and-responses
```

`--write-fills` is rejected because the node documents that it overrides
`--write-trades`; accepting both would bind a command that does not emit the
required trade-row contract.

When `output_file_buffering` is `disabled`, argv must contain exactly one
`--disable-output-file-buffering`. When it is `enabled`, that flag must be
absent. The complete argv remains hash-bound; these checks are only structural
minimums and do not qualify the claim.

## Recording-file invariants

- At least one committed and one trade file are required.
- Paths are unique, relative, slash-separated, and may not contain empty,
  current-directory, parent-directory, backslash, absolute, or control
  components.
- `(role, rotation_sequence)` is unique.
- Descriptors are ordered by role, rotation sequence, then path.
- A file's first and last cursor use one epoch and one role-compatible position
  kind. Committed block heights are nondecreasing. Trade end-offsets are
  nonzero, nondecreasing, and at or before retained file size.

These invariants retain raw facts; they do not define block completeness,
transaction identity, match order, zero-trade behavior, or a committed join.
Those semantics remain gated on an approved same-build recording.

## Stable failures

| Reason code | Disposition | Meaning |
|---|---|---|
| `source_join.empty_qualification_manifest` | quarantine | no manifest bytes |
| `source_join.qualification_manifest_too_large` | stop | preflight byte bound exceeded |
| `source_join.qualification_manifest_digest_mismatch` | quarantine | expected and computed SHA-256 differ |
| `source_join.invalid_qualification_manifest` | quarantine | schema, field, digest, flag, path, cursor, or collection invariant failed |
| `source_join.noncanonical_qualification_manifest` | quarantine | parsed JSON differs from canonical re-encoding |
| `source_join.unqualified_source_profile` | stop | canonical digest is absent from the built-in registry |

The existing trusted synthetic empty-block path is unchanged. Future joined
trade admission must require the opaque token in addition to the existing
source-trust boundary.
