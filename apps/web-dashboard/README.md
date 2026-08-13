# Alpha Desk web dashboard

Read-only operator/research watch UI for `hl-api`. It is not a trading
client, not Stage 6, and not live-source qualification.

From the repository root:

```sh
cargo +1.97.1 run -p hl-api --locked --offline -- run --config config/api.example.toml
just web-dashboard
```

The Vite dev server listens on `127.0.0.1:5174` and proxies `/healthz`,
`/readyz`, and `/v1` to `http://127.0.0.1:8788`. Optional
`VITE_HL_API_ORIGIN` and `VITE_HL_API_BEARER` override the proxy target and
credential-mode bearer.
