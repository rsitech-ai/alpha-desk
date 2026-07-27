# Stage 6 Internal Desk Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver an authenticated read-only REST/WebSocket API and a native macOS-first SwiftUI analyst desk with market, wallet, entity, signal, evidence, replay, data-health, shadow-portfolio, and decision-journal workflows, plus a constrained iOS companion.

**Architecture:** `hl-api` composes read models from RocksDB, ClickHouse, and PostgreSQL through domain-defined query ports. Service-to-service RPC uses Tonic over mTLS, desk queries use Axum REST/OpenAPI, and live streams use resumable binary Protobuf WebSockets. The Swift application is split into domain, networking, storage, design-system, and feature packages; actors own sessions, streams, cache, and local model state. Canonical server intelligence remains immutable while personal ranking and notes stay local where possible.

**Tech Stack:** Rust 1.97.1, Axum 0.8.x, Tower, Tonic/Prost 0.14.x, Rustls, OIDC/OAuth2 with Kanidm 1.10.x, PostgreSQL 18.4, ClickHouse 26.3 LTS, RocksDB read ports, Swift 6.3 strict concurrency, SwiftUI, Observation, URLSession, Swift Charts, Canvas, GRDB 7.x/SQLite WAL, Core ML for local personalization only, Swift Testing and XCUITest.

## Global Constraints

- Stage 5 tag `stage-5-research` and its gate record must verify before this plan begins.
- V1 API and clients are read-only with respect to Hyperliquid; no trading secret, signer, or order endpoint exists.
- Every response carrying market/account/signal state includes `as_of`, `block_height`, `data_health`, and `schema_version` where relevant.
- JSON monetary values are decimal strings; no lossy floating-point round trip is allowed.
- Cursor pagination is mandatory for event streams; offset pagination is forbidden there.
- WebSocket streams use server sequence numbers and resume cursors; sequence regression is fatal to the client session.
- Stale cached data is visibly labeled with age and last block; it is never presented as live.
- OIDC access tokens are short-lived, privileged roles require passkeys/WebAuthn, and service identity uses mTLS.
- Authorization is enforced at handler and query layers, not only in the UI.
- Every signal view can drill to the complete evidence bundle and provenance.
- UI networking, persistence, and local model operations are actor-isolated; views do not perform networking directly.
- Core ML may personalize alert ranking only; it may not alter canonical direction, expected return, confidence, or risk.
- Strict local-only mode cannot promise background iOS delivery while suspended; the UI and documentation state this explicitly.
- Full keyboard access, reduced motion, non-color-only status, textual chart summaries, and stale/degraded-state tests are required.
- Every task follows TDD and ends in a focused commit.

---

### Task 1: Define API read ports, public contracts, and OpenAPI generation

**Files:**
- Create: `crates/api-contracts/src/rest.rs`
- Create: `crates/api-contracts/src/stream.rs`
- Create: `crates/api-contracts/src/errors.rs`
- Create: `crates/api-contracts/src/pagination.rs`
- Create: `crates/storage-ports/src/query.rs`
- Create: `schemas/openapi/v1/openapi.yaml`
- Create: `schemas/proto/api/v1/query.proto`
- Create: `schemas/proto/api/v1/stream.proto`
- Create: `crates/api-contracts/tests/json_exactness.rs`
- Create: `crates/api-contracts/tests/schema_examples.rs`
- Create: `docs/contracts/api-v1.md`

**Interfaces:**
- Consumes: stable market/account/entity/signal/research/health domain types.
- Produces: storage-neutral query ports, exact REST DTOs, structured errors, cursor types, stream envelope, Protobuf/gRPC contracts, and generated OpenAPI examples.

- [ ] **Step 1: Verify Stage 5 and write decimal/metadata contract tests**

```bash
git verify-tag stage-5-research
just stage-5-gate
```

Create a signal DTO with a very large decimal and assert JSON encodes it as an exact string. Assert every state response type contains `as_of`, `block_height`, `data_health`, and `schema_version`.

- [ ] **Step 2: Define common response metadata and errors**

```rust
pub struct ResponseMeta {
    pub as_of_micros: i64,
    pub block_height: String,
    pub data_health: HealthState,
    pub schema_version: String,
    pub trace_id: String,
}

pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
    pub trace_id: String,
    pub details: BTreeMap<String, String>,
}
```

Error codes are stable and documented. Internal database/model details never appear in client messages.

- [ ] **Step 3: Define cursor and stream contracts**

Cursors are opaque, signed, versioned values containing query fingerprint, last sort key, snapshot watermark, and expiry. Stream envelope fields exactly match the approved design and production uses binary Protobuf.

- [ ] **Step 4: Define read/query ports**

Create focused traits such as `MarketQuery`, `AccountQuery`, `EntityQuery`, `SignalQuery`, `ReplayQuery`, `ModelQuery`, `PortfolioQuery`, and `DecisionQuery`. Methods accept explicit authorization scope, `committed_only`, as-of context, pagination, and resource budgets; they expose no ClickHouse/RocksDB/PostgreSQL types.

- [ ] **Step 5: Generate and validate OpenAPI/Protobuf examples**

Every core endpoint from design section 20.3 appears in OpenAPI. Examples pass schema validation and decimal exactness tests.

```bash
cargo test -p api-contracts
cargo run -p schema-check -- check schemas/proto/baseline/v1.pb target/schema/current.pb
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/api-contracts crates/storage-ports/src/query.rs schemas/openapi schemas/proto/api docs/contracts/api-v1.md Cargo.toml Cargo.lock
git commit -m "feat(api): define exact read and stream contracts"
```

---

### Task 2: Implement query repositories and bounded data composition

**Files:**
- Create: `services/hl-api/src/repository/mod.rs`
- Create: `services/hl-api/src/repository/hot_state.rs`
- Create: `services/hl-api/src/repository/clickhouse.rs`
- Create: `services/hl-api/src/repository/postgres.rs`
- Create: `services/hl-api/src/repository/composite.rs`
- Create: `services/hl-api/src/repository/budget.rs`
- Create: `services/hl-api/tests/repository.rs`
- Create: `config/api-query-budgets.toml`
- Create: `docs/api/query-planning.md`

**Interfaces:**
- Consumes: RocksDB read-only state/checkpoint access, ClickHouse analytical tables, PostgreSQL control tables, data-health service, and query authorization context.
- Produces: bounded read-port implementations with consistent watermarks, source provenance, caching rules, timeouts, and degraded behavior.

- [ ] **Step 1: Write mixed-watermark and budget tests**

Create hot state at block 1,000 and analytical features at block 995. Assert the composite response either uses a consistent 995 watermark or explicitly marks the lagging component; it may not imply block 1,000 consistency. Assert over-budget graph/history queries fail with `QUERY_BUDGET_EXCEEDED`.

- [ ] **Step 2: Implement read-only adapters**

RocksDB opens through a read-only checkpoint/snapshot interface and never from an API process against the live write directory unless supported by the approved snapshot mechanism. ClickHouse uses query settings with max rows/bytes/time and a read-only user. PostgreSQL uses prepared statements and role-scoped views.

- [ ] **Step 3: Implement consistent query composition**

`CompositeQueryContext` selects a common watermark, health dependencies, schema versions, and provenance. If ClickHouse is unavailable, hot state endpoints continue while historical endpoints return a structured degraded response. PostgreSQL outage disables configuration writes and may allow short-lived existing sessions under policy.

- [ ] **Step 4: Add cache policy**

Cache only immutable or watermark-keyed results. Never cache authorization decisions beyond token/session policy. Every cache value includes query fingerprint, watermark, schema, health, and expiry.

- [ ] **Step 5: Verify representative query plans**

```bash
just dev-up
cargo test -p hl-api repository
cargo run -p hl-api -- explain-query market-sentiment --market perp:validator:BTC
just dev-down
```

Expected: bounded queries use intended sort/order keys and return consistent metadata.

- [ ] **Step 6: Commit**

```bash
git add services/hl-api/src/repository services/hl-api/tests/repository.rs config/api-query-budgets.toml docs/api/query-planning.md
git commit -m "feat(api): compose bounded hot and analytical reads"
```

---

### Task 3: Implement OIDC, mTLS, RBAC, device sessions, and audit logging

**Files:**
- Create: `services/hl-api/src/auth/mod.rs`
- Create: `services/hl-api/src/auth/oidc.rs`
- Create: `services/hl-api/src/auth/mtls.rs`
- Create: `services/hl-api/src/auth/rbac.rs`
- Create: `services/hl-api/src/auth/session.rs`
- Create: `services/hl-api/src/audit.rs`
- Create: `services/hl-api/tests/auth.rs`
- Create: `schemas/postgres/0007_identity_sessions_audit.sql`
- Create: `infra/podman/quadlet/kanidm.container`
- Create: `infra/ansible/roles/kanidm/defaults/main.yml`
- Create: `infra/ansible/roles/kanidm/tasks/main.yml`
- Create: `infra/ansible/roles/kanidm/handlers/main.yml`
- Create: `infra/ansible/roles/kanidm/templates/server.toml.j2`
- Create: `config/auth.example.toml`
- Create: `docs/security/authentication-authorization.md`

**Interfaces:**
- Consumes: standard OIDC discovery/JWKS, mTLS service certificates, device registrations, role/permission policy, tamper-evident audit port.
- Produces: authenticated principals, service identities, handler/query authorization decisions, short-lived sessions, passkey-required privileged roles, and append-only privileged audit events.

- [ ] **Step 1: Write authorization matrix tests**

Test roles `viewer`, `analyst`, `researcher`, `risk`, `operator`, `admin`, and `auditor` against every endpoint/action class. Assert UI-hidden actions are still rejected server-side. Test expired token, wrong audience/issuer, stale JWKS, revoked device, missing mTLS, and privilege escalation attempts.

- [ ] **Step 2: Implement strict OIDC validation**

Validate issuer, audience, signature, expiry/not-before, nonce/state where applicable, token type, role/group claims, and device/session binding. JWKS cache has bounded TTL and fail-closed rules. Privileged roles require an authentication-method reference consistent with passkey/WebAuthn policy.

- [ ] **Step 3: Implement service mTLS and RBAC**

Map SPIFFE-like or configured certificate SANs to service roles. mTLS and user token are both required for administrative service routes. Authorization decisions are pure functions over principal, permission, resource, and context and are repeated in query repositories.

- [ ] **Step 4: Implement tamper-evident audit records**

Each audit record contains sequence, previous hash, timestamp, principal/device/service, action, resource, request/response classification, artifact/config hash, result, reason, and trace ID. Batch anchors are signed or replicated to write-once archive; verification is automated.

- [ ] **Step 5: Verify Kanidm deployment and auth flow in integration**

```bash
just dev-up
cargo test -p hl-api auth
cargo run -p hl-api -- verify-audit-chain --from 1
just dev-down
```

Expected: all role cases pass and audit chain verifies.

- [ ] **Step 6: Commit**

```bash
git add services/hl-api/src/auth services/hl-api/src/audit.rs services/hl-api/tests/auth.rs schemas/postgres/0007_identity_sessions_audit.sql infra/podman/quadlet/kanidm.container infra/ansible/roles/kanidm config/auth.example.toml docs/security/authentication-authorization.md
git commit -m "feat(security): add OIDC mTLS RBAC and audit chain"
```

---

### Task 4: Implement REST endpoints, control metadata, and export jobs

**Files:**
- Create: `services/hl-api/src/http/mod.rs`
- Create: `services/hl-api/src/http/router.rs`
- Create: `services/hl-api/src/http/middleware.rs`
- Create: `services/hl-api/src/http/system.rs`
- Create: `services/hl-api/src/http/markets.rs`
- Create: `services/hl-api/src/http/accounts.rs`
- Create: `services/hl-api/src/http/entities.rs`
- Create: `services/hl-api/src/http/signals.rs`
- Create: `services/hl-api/src/http/research.rs`
- Create: `services/hl-api/src/http/control.rs`
- Create: `services/hl-api/src/http/export.rs`
- Create: `services/hl-api/tests/http_contract.rs`
- Create: `schemas/postgres/0008_watchlists_alerts_portfolios_decisions.sql`

**Interfaces:**
- Consumes: query ports, auth/RBAC, OpenAPI DTOs, audit, watchlist/alert/portfolio/decision control store, Arrow/Parquet export writer.
- Produces: every approved `/v1` endpoint, exact validation/error behavior, idempotent control writes, and bounded authenticated exports.

- [ ] **Step 1: Write contract tests from OpenAPI examples**

For every endpoint, test success, validation failure, unauthorized, forbidden, not found, stale/degraded data, pagination, and committed-only behavior. Deserialize server responses through the generated client schema.

- [ ] **Step 2: Build the Axum/Tower router and middleware order**

Order: trace/request ID, body/timeout limits, network policy, authentication, authorization context, rate/resource budget, handler, audit, response compression where safe. Administrative/control routes include CSRF defenses when browser flows are enabled.

- [ ] **Step 3: Implement read endpoints by feature slice**

Implement system, market, account, entity, signal/evidence/outcome, replay, model/experiment, and execution-estimate endpoints exactly as approved. `POST /v1/execution-estimates` is a read-only simulation query, not order placement.

- [ ] **Step 4: Implement idempotent control writes**

Watchlist, alert rule, virtual portfolio, shadow allocation, and decision writes require client request ID and optimistic version. Decision records link exact evidence bundle/portfolio snapshot hashes and are append-only except explicit versioned amendments.

- [ ] **Step 5: Implement authenticated bulk exports**

Exports create immutable Arrow IPC/Parquet jobs with query hash, watermark, schema, row/byte cap, requester, expiry, and artifact hash. Download uses a short-lived internal token and audit record.

- [ ] **Step 6: Verify and commit**

```bash
just dev-up
cargo test -p hl-api --test http_contract
cargo run -p hl-api -- print-openapi > target/openapi.yaml
diff -u schemas/openapi/v1/openapi.yaml target/openapi.yaml
just dev-down
```

Expected: contract and schema diff pass.

```bash
git add services/hl-api/src/http services/hl-api/tests/http_contract.rs schemas/postgres/0008_watchlists_alerts_portfolios_decisions.sql
git commit -m "feat(api): expose authenticated read and desk-control endpoints"
```

---

### Task 5: Implement resumable WebSocket streams and alert delivery

**Files:**
- Create: `services/hl-api/src/stream/mod.rs`
- Create: `services/hl-api/src/stream/session.rs`
- Create: `services/hl-api/src/stream/resume.rs`
- Create: `services/hl-api/src/stream/fanout.rs`
- Create: `services/hl-api/src/stream/backpressure.rs`
- Create: `services/hl-api/src/alerts/mod.rs`
- Create: `services/hl-api/src/alerts/rules.rs`
- Create: `services/hl-api/src/alerts/budget.rs`
- Create: `services/hl-api/tests/websocket.rs`
- Create: `docs/contracts/websocket-v1.md`

**Interfaces:**
- Consumes: live NATS/read-model updates, stream authorization, resume cursor, watchlists/alert rules, material-change policy, health.
- Produces: authorized channels, monotonically sequenced Protobuf envelopes, snapshot/resume behavior, bounded per-client queues, and local network alert events.

- [ ] **Step 1: Write reconnect, cursor-expiry, and slow-client tests**

Test disconnect after sequence 100, resume from 100, no duplicates/gaps, expired cursor requiring snapshot, unauthorized subscription, sequence regression rejection, and slow client queue overflow. Invalidation/data-health red must bypass normal cooldown.

- [ ] **Step 2: Implement stream channels and sequence store**

Channels exactly match the design. Each persisted stream event has global channel sequence, source watermark, schema, health, payload type, and content hash. Resume tokens are signed and bound to principal/channel/filter.

- [ ] **Step 3: Implement bounded fan-out/backpressure**

Per-client queues are bounded by messages/bytes/time. Low-priority updates may coalesce to the latest state; lifecycle invalidation and critical health events are never silently dropped. An overwhelmed client is closed with a resync-required reason and audit/metric.

- [ ] **Step 4: Implement alert rules and fatigue budget**

Rules evaluate canonical signal/health fields and personal watchlists/portfolio context. One evolving thread, material-change threshold, category cooldown, and daily budget apply; invalidations and critical health bypass budget. Delivery targets are active app streams and optional on-premise gateways only in strict local mode.

- [ ] **Step 5: Verify binary and diagnostic modes**

```bash
cargo test -p hl-api --test websocket
```

Expected: Protobuf and non-production JSON diagnostic envelopes are semantically equivalent and all resume tests pass.

- [ ] **Step 6: Commit**

```bash
git add services/hl-api/src/stream services/hl-api/src/alerts services/hl-api/tests/websocket.rs docs/contracts/websocket-v1.md
git commit -m "feat(api): stream resumable live intelligence and alerts"
```

---

### Task 6: Integrate and harden the `hl-api` service

**Files:**
- Modify: `services/hl-api/src/main.rs`
- Create: `services/hl-api/src/app.rs`
- Create: `services/hl-api/src/config.rs`
- Create: `services/hl-api/src/grpc.rs`
- Create: `services/hl-api/src/health.rs`
- Create: `services/hl-api/tests/end_to_end.rs`
- Create: `config/api.example.toml`
- Create: `infra/systemd/hl-api.service.d/override.conf`
- Create: `infra/monitoring/dashboards/api.json`
- Create: `infra/monitoring/alerts/api.yml`
- Create: `docs/runbooks/api-restart.md`

**Interfaces:**
- Consumes: repositories, auth, HTTP, streams, alerts, gRPC service clients, telemetry/config.
- Produces: production-ready `hl-api` with graceful shutdown, readiness/degraded behavior, request budgets, TLS, metrics, and restart/resume guarantees.

- [ ] **Step 1: Write end-to-end API/stream scenario**

Authenticate a test analyst, load command-center data, subscribe to signal lifecycle, disconnect/reconnect, create a shadow decision, retrieve attribution placeholder state, degrade ClickHouse, and assert hot state/streams remain correct with explicit historical degradation.

- [ ] **Step 2: Implement startup dependency policy**

OIDC/JWKS, PostgreSQL, required service identities, TLS keys, schema compatibility, and hot query source must pass before readiness. ClickHouse may be degraded after startup under policy. No config includes trading credentials or execution endpoint.

- [ ] **Step 3: Implement graceful shutdown and drain**

Stop accepting new HTTP/WS sessions, send reconnect advisory, drain in-flight requests within budget, persist stream sequence and audit writes, then close pools. Hard timeout exits non-zero and is tested.

- [ ] **Step 4: Measure API SLOs**

Benchmark hot query p95 <150 ms/p99 <500 ms, standard historical p95 <3 s, signal-to-macOS healthy LAN/VPN p99 <200 ms, active stream fan-out, and graph/history budgets.

- [ ] **Step 5: Verify security headers and no execution surface**

Automated route inventory asserts no route contains order/place/cancel/sign/withdraw execution semantics and no binary links an execution/signing crate.

- [ ] **Step 6: Commit**

```bash
git add services/hl-api config/api.example.toml infra/systemd/hl-api.service.d infra/monitoring/dashboards/api.json infra/monitoring/alerts/api.yml docs/runbooks/api-restart.md
git commit -m "feat(api): integrate hardened internal desk service"
```

---

### Task 7: Build Swift domain, networking, stream, and local-storage foundations

**Files:**
- Modify: `apps/AlphaDesk/Package.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/ExactDecimal.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/Identifiers.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/DataHealth.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/Market.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/Wallet.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/Signal.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/Portfolio.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/Decision.swift`
- Create: `apps/AlphaDesk/Sources/DeskDomain/StreamEnvelope.swift`
- Create: `apps/AlphaDesk/Sources/DeskNetworking/APIClient.swift`
- Create: `apps/AlphaDesk/Sources/DeskNetworking/SessionActor.swift`
- Create: `apps/AlphaDesk/Sources/DeskNetworking/StreamActor.swift`
- Create: `apps/AlphaDesk/Sources/DeskStorage/DatabaseActor.swift`
- Create: `apps/AlphaDesk/Sources/DeskStorage/Migrations.swift`
- Create: `apps/AlphaDesk/Sources/DeskStorage/Records/CachedMarketRecord.swift`
- Create: `apps/AlphaDesk/Sources/DeskStorage/Records/CachedSignalRecord.swift`
- Create: `apps/AlphaDesk/Sources/DeskStorage/Records/DecisionRecord.swift`
- Create: `apps/AlphaDesk/Sources/DeskStorage/Records/StreamCursorRecord.swift`
- Create: `apps/AlphaDesk/Tests/DeskDomainTests/ExactDecimalTests.swift`
- Create: `apps/AlphaDesk/Tests/DeskDomainTests/EnvelopeDecodingTests.swift`
- Create: `apps/AlphaDesk/Tests/DeskNetworkingTests/StreamResumeTests.swift`
- Create: `apps/AlphaDesk/Tests/DeskNetworkingTests/SessionActorTests.swift`
- Create: `apps/AlphaDesk/Tests/DeskStorageTests/MigrationTests.swift`
- Create: `apps/AlphaDesk/Tests/DeskStorageTests/CacheFreshnessTests.swift`
- Create: `apps/AlphaDesk/Resources/api-v1.json`

**Interfaces:**
- Consumes: OpenAPI/Protobuf contracts, OIDC endpoints, WebSocket resume protocol, exact decimal strings, local cache schema.
- Produces: Sendable Swift domain snapshots, authenticated async API client, one actor-owned stream connection/resume cursor, GRDB WAL cache/migrations, stale-state metadata, and generated contract fixtures.

- [ ] **Step 1: Write exact decimal and stale-state tests**

Use a decimal beyond binary floating precision and assert round-trip through JSON/domain/SQLite remains exact. Cache a response, advance wall time, and assert `FreshnessState.stale(age:lastBlock:)` is produced and visible to feature modules.

- [ ] **Step 2: Define immutable Sendable domain types**

```swift
public enum DecimalStringError: Error, Equatable {
    case invalidSyntax
}

public struct DecimalString: Sendable, Hashable, Codable {
    public let rawValue: String

    public init(_ rawValue: String) throws {
        guard !rawValue.isEmpty,
              rawValue == rawValue.trimmingCharacters(in: .whitespacesAndNewlines)
        else { throw DecimalStringError.invalidSyntax }

        let unsigned: Substring
        if rawValue.first == "-" {
            unsigned = rawValue.dropFirst()
        } else {
            unsigned = Substring(rawValue)
        }
        guard !unsigned.isEmpty else { throw DecimalStringError.invalidSyntax }
        let parts = unsigned.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 1 || parts.count == 2,
              !parts[0].isEmpty,
              parts[0].allSatisfy(\.isNumber),
              parts.count == 1 || (!parts[1].isEmpty && parts[1].allSatisfy(\.isNumber))
        else { throw DecimalStringError.invalidSyntax }

        self.rawValue = rawValue
    }
}

public struct SnapshotMetadata: Sendable, Hashable, Codable {
    public let asOf: Date
    public let blockHeight: String
    public let dataHealth: DataHealth
    public let schemaVersion: String
}
```

No domain model stores money as `Double`.

- [ ] **Step 3: Implement `SessionActor` and API client**

`SessionActor` owns OIDC tokens, refresh, device/session state, logout, and Keychain references. `APIClient` accepts a `SessionProviding` actor and typed endpoint request; UI views never access tokens or URLSession directly.

- [ ] **Step 4: Implement `StreamActor`**

One actor owns the WebSocket, subscription set, last acknowledged sequence per channel, reconnect backoff, resume token, snapshot fallback, and message decode. Sequence regression clears the affected cache and requires full resync.

- [ ] **Step 5: Implement GRDB storage actor and migrations**

Use SQLite WAL, explicit schema version, transactionally stored snapshots/stream cursors/watchlists/decisions/local preferences, and tested forward migrations. Cached server intelligence is keyed by watermark/schema/query and never overwrites newer data.

- [ ] **Step 6: Run Swift strict-concurrency and migration tests**

```bash
swift test --package-path apps/AlphaDesk
swift build --package-path apps/AlphaDesk -Xswiftc -strict-concurrency=complete
```

Expected: PASS with no controllable strict-concurrency warnings.

- [ ] **Step 7: Commit**

```bash
git add apps/AlphaDesk
git commit -m "feat(swift): add domain networking stream and cache actors"
```

---

### Task 8: Build the macOS app shell, design system, Command Center, and Market Detail

**Files:**
- Create: `apps/AlphaDesk/App/AlphaDeskApp.swift`
- Create: `apps/AlphaDesk/App/AppShell.swift`
- Create: `apps/AlphaDesk/Packages/DeskDesignSystem/Package.swift`
- Create: `apps/AlphaDesk/Packages/DeskDesignSystem/Sources/DeskDesignSystem/MetricCard.swift`
- Create: `apps/AlphaDesk/Packages/DeskDesignSystem/Sources/DeskDesignSystem/HealthBadge.swift`
- Create: `apps/AlphaDesk/Packages/MarketCommandCenter/Package.swift`
- Create: `apps/AlphaDesk/Packages/MarketCommandCenter/Sources/MarketCommandCenter/MarketCommandCenterView.swift`
- Create: `apps/AlphaDesk/Packages/MarketCommandCenter/Sources/MarketCommandCenter/MarketCommandCenterModel.swift`
- Create: `apps/AlphaDesk/Packages/MarketDetail/Package.swift`
- Create: `apps/AlphaDesk/Packages/MarketDetail/Sources/MarketDetail/MarketDetailView.swift`
- Create: `apps/AlphaDesk/Packages/MarketDetail/Sources/MarketDetail/FragilityScenarioView.swift`
- Create: `apps/AlphaDesk/Packages/DataHealthUI/Package.swift`
- Create: `apps/AlphaDesk/Packages/DataHealthUI/Sources/DataHealthUI/DataHealthView.swift`
- Create: `apps/AlphaDesk/Tests/MarketCommandCenterTests/MarketCommandCenterSnapshotTests.swift`
- Create: `apps/AlphaDesk/Tests/MarketDetailTests/MarketDetailSnapshotTests.swift`
- Create: `apps/AlphaDesk/UITests/CommandCenterUITests.swift`

**Interfaces:**
- Consumes: Swift domain/network/storage packages and command-center/market REST/stream contracts.
- Produces: macOS navigation shell, session/data-health chrome, keyboard command palette, evidence-drillable command center, and full market intelligence views.

- [ ] **Step 1: Write view-model state tests before views**

Test loading, fresh, stale, amber, red, empty, partial, and error states. Assert red required data suppresses actionable styling and exposes the health reason. Assert cards retain selected market/timestamp across navigation.

- [ ] **Step 2: Implement a restrained professional design system**

Provide typography, spacing, table, status badge, metric card, confidence interval, evidence link, empty/error/stale state, keyboard focus, textual chart summary, and numeric alignment components. Color never carries meaning alone.

- [ ] **Step 3: Implement AppShell and operator navigation**

Primary macOS split navigation includes Command Center, Markets, Wallets, Entities, Signals, Replay, Portfolios, Decisions, Research, and Data Health. Session/global health are always visible. Command palette and keyboard navigation cover all primary destinations.

- [ ] **Step 4: Implement Command Center**

Cards: regime map, Smart Flow/horizon, smart-crowd divergence, independent consensus, fragility asymmetry, liquidity/spread stress, carry pressure, canonical/personalized signal ranking, data health, and portfolio exposure. Every card opens evidence or relevant detail.

- [ ] **Step 5: Implement Market Detail**

Views include price/volume context, full sentiment vector, scoped ratio metadata, new-risk decomposition, entry/pain map, leverage/liquidation distribution, fragility curve, cohort activity, analogues, signals/invalidations, and impact/capacity curves.

- [ ] **Step 6: Add previews, snapshots, accessibility, and performance tests**

Use deterministic fixtures for large/small/missing values, dark/light, reduced motion, keyboard, VoiceOver labels, textual summaries, and 1,000-row tables. Run Swift tests and UI tests on supported macOS simulator/host.

- [ ] **Step 7: Commit**

```bash
git add apps/AlphaDesk/App apps/AlphaDesk/Packages/DeskDesignSystem apps/AlphaDesk/Packages/MarketCommandCenter apps/AlphaDesk/Packages/MarketDetail apps/AlphaDesk/Packages/DataHealthUI apps/AlphaDesk/Tests apps/AlphaDesk/UITests/CommandCenterUITests.swift
git commit -m "feat(mac): build command center and market intelligence views"
```

---

### Task 9: Build Wallet DNA, entity graph, intelligence tape, signal evidence, and replay

**Files:**
- Create: `apps/AlphaDesk/Packages/WalletDNA/Package.swift`
- Create: `apps/AlphaDesk/Packages/WalletDNA/Sources/WalletDNA/WalletDNAView.swift`
- Create: `apps/AlphaDesk/Packages/EntityGraphUI/Package.swift`
- Create: `apps/AlphaDesk/Packages/EntityGraphUI/Sources/EntityGraphUI/EntityGraphView.swift`
- Create: `apps/AlphaDesk/Packages/IntelligenceTape/Package.swift`
- Create: `apps/AlphaDesk/Packages/IntelligenceTape/Sources/IntelligenceTape/IntelligenceTapeView.swift`
- Create: `apps/AlphaDesk/Packages/SignalEvidence/Package.swift`
- Create: `apps/AlphaDesk/Packages/SignalEvidence/Sources/SignalEvidence/SignalEvidenceView.swift`
- Create: `apps/AlphaDesk/Packages/ReplayUI/Package.swift`
- Create: `apps/AlphaDesk/Packages/ReplayUI/Sources/ReplayUI/ReplayView.swift`
- Create: `apps/AlphaDesk/Tests/WalletDNATests/WalletDNASnapshotTests.swift`
- Create: `apps/AlphaDesk/Tests/EntityGraphUITests/EntityGraphSnapshotTests.swift`
- Create: `apps/AlphaDesk/Tests/SignalEvidenceTests/SignalEvidenceCompletenessTests.swift`
- Create: `apps/AlphaDesk/Tests/ReplayUITests/FutureOutcomeHidingTests.swift`
- Create: `apps/AlphaDesk/UITests/AnalystDrilldownUITests.swift`

**Interfaces:**
- Consumes: account/entity/signal/evidence/replay APIs and streams, stable graph neighborhoods, as-of context.
- Produces: complete professional wallet/entity analysis, evidence-first tape, auditable signal view, and whole-app historical time-machine context.

- [ ] **Step 1: Write point-in-time navigation and evidence tests**

Enter replay at block N, open wallet/entity/signal, and assert every request carries the same replay context and future outcomes remain hidden. Signal view must show every evidence category required by the server contract.

- [ ] **Step 2: Implement Wallet DNA**

Show account type/mode, cash-flow-adjusted equity/performance, realized/unrealized/fees/funding, drawdown/tail risk, skill intervals, style/behavior, regime/asset/horizon performance, markouts, copyability by bankroll, entity membership, leader/follower, counterparties, and change points.

- [ ] **Step 3: Implement stable entity graph rendering**

Server supplies bounded neighborhood and stable layout hints. Hard/inferred edges differ textually and visually; thickness means confidence. Time slider changes cluster version. Edge inspection shows evidence, confidence, alternatives, and no verified-identity claim. Layout does not re-force on every update.

- [ ] **Step 4: Implement intelligence tape and signal evidence**

Tape items narrate change, subject, new/closing/hedging risk, skill/independence/regime fit, half-life/capacity, historical outcomes, health, and invalidations. Signal view shows lifecycle, cost/edge, model/build/feature versions, triggering events, entities/weights, before/after features, analogues, assumptions, and limitations.

- [ ] **Step 5: Implement whole-app replay context**

Replay actor controls block/time, speed, step-by-block, pause, and hidden-outcome policy. Current-methodology comparison is visibly labeled and separated from what was known then.

- [ ] **Step 6: Verify large graph/history performance and accessibility**

Test 5,000-node server-side dataset with bounded 100–300 node neighborhoods, 100,000-row wallet history through pagination/virtualization, keyboard graph navigation, textual edge summaries, and stale/degraded states.

- [ ] **Step 7: Commit**

```bash
git add apps/AlphaDesk/Packages/WalletDNA apps/AlphaDesk/Packages/EntityGraphUI apps/AlphaDesk/Packages/IntelligenceTape apps/AlphaDesk/Packages/SignalEvidence apps/AlphaDesk/Packages/ReplayUI apps/AlphaDesk/Tests apps/AlphaDesk/UITests/AnalystDrilldownUITests.swift
git commit -m "feat(mac): add wallet entity evidence and replay workflows"
```

---

### Task 10: Build shadow portfolios, decision journal, attribution, and personal ranking

**Files:**
- Create: `apps/AlphaDesk/Packages/PortfolioRisk/Package.swift`
- Create: `apps/AlphaDesk/Packages/PortfolioRisk/Sources/PortfolioRisk/PortfolioRiskView.swift`
- Create: `apps/AlphaDesk/Packages/DecisionJournal/Package.swift`
- Create: `apps/AlphaDesk/Packages/DecisionJournal/Sources/DecisionJournal/DecisionJournalView.swift`
- Create: `apps/AlphaDesk/Packages/AlertPersonalization/Package.swift`
- Create: `apps/AlphaDesk/Packages/AlertPersonalization/Sources/AlertPersonalization/AlertRanker.swift`
- Create: `apps/AlphaDesk/Tests/PortfolioRiskTests/PortfolioRiskTests.swift`
- Create: `apps/AlphaDesk/Tests/DecisionJournalTests/DecisionAmendmentTests.swift`
- Create: `apps/AlphaDesk/Tests/AlertPersonalizationTests/CanonicalInvarianceTests.swift`
- Create: `services/hl-api/src/portfolio_risk.rs`
- Create: `services/hl-api/src/attribution.rs`
- Create: `services/hl-api/tests/portfolio_decision.rs`
- Create: `models/test-models/coreml-alert-ranking/AlertRanker.mlpackage/Manifest.json`
- Create: `models/test-models/coreml-alert-ranking/golden-input.json`
- Create: `models/test-models/coreml-alert-ranking/golden-output.json`
- Create: `docs/product/decision-attribution.md`

**Interfaces:**
- Consumes: owned read-only addresses, virtual capital, shadow allocations, canonical signals/evidence, execution estimates, portfolio factors, decision records, manually entered or observed fills, local reviewed labels.
- Produces: portfolio-adjusted personal signal ranking, risk/concentration views, append-only decisions, and signal/selection/sizing/execution/process attribution.

- [ ] **Step 1: Write portfolio correlation and decision immutability tests**

Create two signals driven by the same entity cluster/factor and assert combined marginal utility is penalized. Assert changing a submitted decision creates an amendment linked to the original, preserving original evidence/portfolio hashes.

- [ ] **Step 2: Implement server portfolio risk composition**

Calculate gross/net/beta/asset/entity-cluster/strategy/regime/leverage/liquidity/fragility exposure, marginal expected return/tail risk/correlation/capacity, and concentration. Owned addresses are read-only identifiers; no private key fields exist.

- [ ] **Step 3: Implement decision records and outcome visibility**

Implement all approved `DecisionRecord` fields with enum actions/reason codes, thesis, entry/exit/invalidation, intended size, actual fill reference, timestamps, and replay outcome visibility. Link exact evidence and portfolio snapshot hashes.

- [ ] **Step 4: Implement attribution**

Separate signal quality, selection quality, sizing quality, execution quality, and process quality. Profitable outcome does not imply good process; a correct signal with poor execution is attributed accordingly. Free-form notes never enter canonical models.

- [ ] **Step 5: Implement local Core ML personalization only**

A signed local model may reorder alerts using reviewed accepted/dismissed/acted-on reason codes and personal behavior. The UI clearly labels personalized order. Tests assert canonical expected return, direction, confidence, and risk remain byte-for-byte unchanged.

- [ ] **Step 6: Verify and commit**

```bash
cargo test -p hl-api portfolio_decision
swift test --package-path apps/AlphaDesk --filter PortfolioRiskTests
swift test --package-path apps/AlphaDesk --filter DecisionJournalTests
swift test --package-path apps/AlphaDesk --filter AlertPersonalizationTests
```

Expected: all tests pass.

```bash
git add services/hl-api/src/portfolio_risk.rs services/hl-api/src/attribution.rs services/hl-api/tests/portfolio_decision.rs apps/AlphaDesk/Packages/PortfolioRisk apps/AlphaDesk/Packages/DecisionJournal apps/AlphaDesk/Packages/AlertPersonalization apps/AlphaDesk/Tests models/test-models/coreml-alert-ranking docs/product/decision-attribution.md
git commit -m "feat(desk): add portfolio risk decisions and local ranking"
```

---

### Task 11: Build the iOS companion and cross-platform operator quality gates

**Files:**
- Create: `apps/AlphaDesk/App/iOS/AlphaDeskIOSApp.swift`
- Create: `apps/AlphaDesk/App/iOS/RootView.swift`
- Create: `apps/AlphaDesk/App/iOS/NotificationDelegate.swift`
- Create: `apps/AlphaDesk/Packages/iOSCompanion/Package.swift`
- Create: `apps/AlphaDesk/Packages/iOSCompanion/Sources/iOSCompanion/CompanionView.swift`
- Create: `apps/AlphaDesk/Packages/iOSCompanion/Sources/iOSCompanion/BackgroundRefreshPolicy.swift`
- Create: `apps/AlphaDesk/Tests/iOSCompanionTests/OfflineStateTests.swift`
- Create: `apps/AlphaDesk/Tests/iOSCompanionTests/BackgroundRefreshPolicyTests.swift`
- Create: `apps/AlphaDesk/UITests/iOSCompanionUITests.swift`
- Create: `docs/product/ios-background-limitations.md`
- Create: `docs/product/operator-workflow.md`
- Create: `tools/ui-fixtures/generate.rs`

**Interfaces:**
- Consumes: shared Swift packages, active WebSocket stream, local cache, local notifications, watchlists, evidence/decision APIs.
- Produces: iPhone/iPad active-session companion, documented suspension limits, local notifications while data is received, offline cached review, and tested analyst operating loop.

- [ ] **Step 1: Write suspended/active/offline-state tests**

Assert active streaming updates and local notifications work while connected; suspended strict-local mode displays no false promise of delivery; offline mode shows cached age/block/health and prevents decisions that require fresh data unless explicitly acknowledged.

- [ ] **Step 2: Implement focused mobile surfaces**

Provide watchlist, top signals, evidence summary, data health, owned-account/portfolio risk, decision review, and replay review. Do not duplicate dense macOS graph/research workflows where they are not ergonomic.

- [ ] **Step 3: Document background limitation in product and settings**

Strict local-only mode states that reliable suspended background delivery is unavailable without APNs. Any future APNs bridge remains disabled and outside this V1 plan.

- [ ] **Step 4: Implement analyst workflow support**

Session open, live triage, session close, weekly review, and governance review are documented and linked to application views. The app surfaces unresolved signals, stale data, and decisions requiring reconciliation.

- [ ] **Step 5: Run cross-platform UI/accessibility/performance suites**

Test macOS/iPhone/iPad, light/dark, Dynamic Type, VoiceOver, keyboard where applicable, reduced motion, large/missing values, stale/red health, reconnect storms, cache migration, large lists, and local model fallback.

- [ ] **Step 6: Commit**

```bash
git add apps/AlphaDesk/App/iOS apps/AlphaDesk/Packages/iOSCompanion apps/AlphaDesk/Tests/iOSCompanionTests apps/AlphaDesk/UITests/iOSCompanionUITests.swift docs/product/ios-background-limitations.md docs/product/operator-workflow.md tools/ui-fixtures
git commit -m "feat(ios): add constrained local companion and operator workflow"
```

---

### Task 12: Execute the Stage 6 internal-desk gate

**Files:**
- Create or modify before verification: `config/stage-gates/stage-6.toml`
- Create before verification: `tests/regression/api/manifest.toml`
- Create before verification: `tests/regression/ui/manifest.toml`
- Create or modify before verification: `docs/reviews/desk-security-review-v1.md`
- Create or modify before verification: `docs/reviews/analyst-workflow-acceptance-v1.md`
- Create or modify before verification: `justfile`
- Generate after verification: `docs/stage-gates/stage-6-internal-desk.evidence.json`
- Generate after verification: `docs/stage-gates/stage-6-internal-desk.md`

**Interfaces:**
- Consumes: the complete stage implementation, approved point-in-time regression material, and prior signed gate evidence.
- Produces: a clean-commit canonical gate report, signed approval record, and signed `stage-6-desk` tag.

- [ ] **Step 1: Freeze the regression and review inputs**

Freeze API and UI fixtures covering every health state, stale cache, large decimal, missing evidence, signal lifecycle state, complex entity graph, account mode, replay hidden outcome, portfolio concentration, decision amendment, and iOS offline/active behavior.

- [ ] **Step 2: Implement the exact gate configuration and tests**

`just stage-6-gate` writes only to ignored `target/stage-gates/stage-6.json` and runs API schema, contract, decimal, and error tests; authentication/RBAC/device/mTLS/audit-chain tests; query-budget and mixed-watermark tests; WebSocket reconnect/resume/backpressure/sequence tests; no-execution route inventory; Swift concurrency, migration, snapshot, UI, accessibility, and performance tests; evidence drill-down; replay future hiding; personalization invariance; stale/degraded behavior; API/stream load; and referenced security/restore evidence.

The gate runner must reject a dirty worktree before any check, record the clean implementation SHA, and fail closed on missing evidence or approvals. Add a configuration test proving every required command and artifact is present.

- [ ] **Step 3: Commit every gate input before verification**

```bash
git add config/stage-gates/stage-6.toml tests/regression/api tests/regression/ui docs/reviews/desk-security-review-v1.md docs/reviews/analyst-workflow-acceptance-v1.md justfile
git commit -m "chore(gate): add Stage 6 internal desk verification inputs"
test -z "$(git status --porcelain)"
git rev-parse HEAD
```

The printed SHA is the immutable implementation commit evaluated by this gate.

- [ ] **Step 4: Run the gate from fresh clean clones on two supported hosts**

```bash
just stage-6-gate
cargo run -p desk-acceptance -- tests/regression/api/manifest.toml tests/regression/ui/manifest.toml --output target/stage-gates/stage-6-acceptance.json
sha256sum target/stage-gates/stage-6.json
```

Expected: PASS; canonical report, state/output hashes, and configured reproducibility views agree across hosts. Host-specific provenance remains recorded but is excluded only from the explicitly defined cross-host comparison projection.

- [ ] **Step 5: Commit evidence, collect approvals, and sign the stage tag**

```bash
cp target/stage-gates/stage-6.json docs/stage-gates/stage-6-internal-desk.evidence.json
cargo run -p stage-gate -- render-record --evidence docs/stage-gates/stage-6-internal-desk.evidence.json --output docs/stage-gates/stage-6-internal-desk.md
git add docs/stage-gates/stage-6-internal-desk.evidence.json docs/stage-gates/stage-6-internal-desk.md
git commit -m "docs(gate): record Stage 6 internal desk evidence"
git tag -s stage-6-desk -m "Stage 6 internal desk gate passed"
git verify-tag stage-6-desk
```

Platform/data, security, product/desk, and independent reviewers must provide the detached approval artifacts referenced by the record. Do not create the tag when a required check, comparison, review, or bounded-limitation statement is missing.

## Stage 6 Exit Criteria

- Every approved REST endpoint and stream channel is contract-tested, authorized, bounded, health/watermark aware, and exact for decimals.
- OIDC, passkey policy, mTLS, RBAC, device sessions, and tamper-evident audit records pass review.
- macOS analysts can reproduce every displayed signal from complete evidence and operate through the documented decision loop.
- WebSocket resume, stale cache, degraded dependencies, and sequence-regression behavior are verified.
- Wallet/entity/market/signal/replay/portfolio/decision/data-health views pass accessibility and performance gates.
- iOS behavior and strict local-only background limitations are explicit and tested.
- Route and binary inventories prove V1 has no execution/signing capability.
- `stage-6-desk` is approved and tagged.
