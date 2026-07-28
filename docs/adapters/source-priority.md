# Source Priority and Trust Admission

## Status

The source-trust admission policy is implemented and exhaustively tested. The
operator-feed, independently complete secondary source, public network clients,
historical client, and mempool transports are not implemented.

This distinction is deliberate. A transport proving that it can receive bytes
does not prove completeness, finality, independence, or permission to advance
canonical state.

## Required source declarations

Every source has two independent configuration fields:

- `trust` states the configured provenance and completeness contract.
- `class` states the kind of observation emitted.

`CaptureConfig` validates the pair before a source opens. Unknown values and
incompatible pairs fail startup. Callers cannot configure a separate
`committed`, `canonical`, or `advance_watermark` flag.

| Trust | Allowed observation classes | Publication lane | May advance committed watermark |
| --- | --- | --- | --- |
| `locally-verified-committed` | committed block; local auxiliary order, book, and ledger evidence | committed candidate for blocks; reconciliation for auxiliary evidence | Blocks only |
| `independent-committed` | committed block; independently sourced auxiliary evidence | committed candidate for blocks; reconciliation for auxiliary evidence | Blocks only |
| `reconciled-snapshot` | snapshot | reconciliation | No |
| `recovery-only` | historical block | recovery | No |
| `third-party-provisional` | public market data or provisional feed | provisional | No |
| `mempool-provisional` | provisional mempool | mempool | No |

The runtime and sequencer must consume `SourceAdmission` and its derived
`PublicationLane`. They must not reproduce this matrix as ad hoc conditionals.
Even a valid committed candidate still requires spool durability, continuity,
and later sequencer policy before canonical publication.

## Evidence currently available

The primary-node adapter is locally tested against provenance-labeled,
normalized official documentation examples. It has not been qualified against
byte-exact output from the deployed operator node.

Local discovery found the RSI Tech `hlscreen` repository at commit
`6deb7f506e90c238b63d9c057f9a477c8f65f662`. That project explicitly describes
its checked Hyperliquid fixtures as synthetic/minimized public REST and
WebSocket shapes. It is useful future input for a public provisional adapter,
but it is neither:

- the proprietary low-latency operator feed named by the design; nor
- an independently operated complete committed source.

No operator-feed parser, secondary committed adapter, or production fixture is
presented in this repository until the exact source contract and
redistribution boundary are supplied.

## Inputs required for the remaining Task 4 transports

Operator-feed integration requires:

1. the owning repository and immutable schema commit;
2. one credential-free byte-exact fixture for every discriminant;
3. confirmation of which structural fields may appear in the public tree;
4. an explicit maximum record size and source cursor/reconnect contract.

Independent committed integration requires:

1. a separately operated complete source;
2. its finality and completeness contract;
3. restart/range semantics and a deterministic test corpus;
4. a divergence runbook proving it cannot silently agree with the primary
   source through shared infrastructure.

Public REST/WebSocket, historical, and mempool clients remain lower-trust
transports. They require bounded request/subscription budgets, response-size
limits, timeouts, cancellation, retry/jitter policy, schema-drift quarantine,
and source-native cursor semantics before they are considered implemented.

## Verification

```sh
cargo +1.97.1 test -p hl-protocol --test source_trust --locked --offline
cargo +1.97.1 test -p hl-capture --test config --locked --offline
```

The protocol test covers all 54 combinations of the six trust values and nine
observation classes. It asserts that exactly two pairings—local committed block
and independent committed block—can produce a committed-watermark candidate.
