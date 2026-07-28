# Parser fuzzing

Run the bounded spool parser target from the repository root:

```sh
cargo +nightly-2026-07-16 fuzz run spool_segment fixtures/spool/valid-v1 -- -max_total_time=60
```

The target accepts arbitrary bytes and requires `validate_segment_bytes` to return a typed result
without panicking, aborting, or reading beyond its input. Seed the corpus with
`fixtures/spool/valid-v1/segment-0000000001.hlsp` after generating or updating the canonical
fixture.
