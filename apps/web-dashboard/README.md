# Alpha Desk web dashboard

Read-only operator/research watch UI for `hl-api`. It is not a trading
client, not Stage 6, not live-source qualification, and not a Stage PASS.

The desk polls `hl-api` and treats typed fail-closed HTTP states as first-class
UI: 503 snapshot missing/invalid, 503 hl-core dead-letter open failures
(`core.deadletter_*`), 400 invalid query, 429/400 query budgets, and 501
unspecified streams. It does not invent fills, orders, or qualification.
Missing optional fields stay omitted; they are not displayed as 0.

From the repository root:

```sh
cargo +1.97.1 run -p hl-api --locked --offline -- run --config config/api.example.toml
just web-dashboard
```

The Vite dev server listens on `127.0.0.1:5174` and proxies `/healthz`,
`/readyz`, and `/v1` to `http://127.0.0.1:8788`. Optional
`VITE_HL_API_ORIGIN` and `VITE_HL_API_BEARER` override the proxy target and
credential-mode bearer.

Each poll also probes `/v1/health?offset=1` and `/v1/health?limit=999999`.
When the listener implements query budgets those return typed 400s; when it
does not, the UI reports that the typed error was not observed instead of
painting a PASS.
