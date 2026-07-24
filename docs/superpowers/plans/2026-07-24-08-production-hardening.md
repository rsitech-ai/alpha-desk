# Production Hardening and V1 Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deploy the approved read-only Hyperliquid Alpha Desk on operator-controlled infrastructure with hardened trust zones, measurable SLOs, tested backups/recovery, load/chaos evidence, signed reproducible releases, and a safe open-source separation strategy.

**Architecture:** The latency-critical capture/core path runs as hardened systemd services on dedicated Ubuntu 24.04 hosts in Tokyo with an independent secondary source. Stateful dependencies run on operator-controlled hosts through systemd/Podman and replicated storage. Acquisition, analytics, user access, and future execution are separate network/security zones; the execution zone remains empty in V1. All operational state is observable locally, and every release is reproducible, signed, SBOM-attached, canaried, and reversible.

**Tech Stack:** Ubuntu 24.04 LTS, Ansible, systemd, Podman/Quadlet, WireGuard, nftables, chrony, ZFS or equivalent resilient archive storage, NATS JetStream, RocksDB, ClickHouse 26.3 LTS, PostgreSQL 18.4, MinIO/S3-compatible archive, Kanidm, OpenTelemetry Collector, Prometheus/VictoriaMetrics, local logs/traces, age/sops or operator-approved secret store, cosign-compatible artifact signing where appropriate, Syft/CycloneDX SBOMs, cargo-deny/audit, Swift dependency verification.

## Global Constraints

- Tags `stage-0-foundations` through `stage-6-desk` and all gate records must verify before production release.
- Primary committed hot path is deployed in Tokyo; an independent secondary capture path uses a separate failure domain.
- Acquisition, analytics core, user access, and future execution are default-deny network zones.
- V1 deploys no `hl-exec`, signer, trading key, order placement, cancellation, withdrawal, or execution gateway artifact.
- Databases, NATS, node outputs, archive, metrics, logs, traces, identity, and model artifacts remain operator-controlled and local.
- Time is UTC with multiple trusted chrony sources; protocol order, not wall clock, determines committed event order.
- ClickHouse is rebuildable, RocksDB is rebuildable, and immutable archives plus manifests/checkpoints are the recovery source.
- A red correctness state suppresses affected intelligence; availability never overrides data correctness.
- Production changes use reviewed Ansible/config commits and signed releases; ad hoc host changes are drift incidents.
- Secrets never enter Git, logs, crash dumps, client caches, or model bundles.
- Security and restore gates require independent review.
- Open-source publication excludes proprietary feed adapters, private alpha configurations, private model/data artifacts, operator identities, and infrastructure secrets while retaining clean plugin contracts.
- Every task follows TDD or infrastructure-as-code verification and ends in a focused commit.

---

### Task 1: Finalize production inventory, capacity model, and hardware acceptance benchmarks

**Files:**
- Create: `infra/ansible/inventory/production/hosts.yml.example`
- Create: `infra/ansible/group_vars/production.yml.example`
- Create: `infra/capacity/capacity-model.toml`
- Create: `tools/capacity-plan/src/main.rs`
- Create: `tools/hardware-acceptance/run.sh`
- Create: `tools/hardware-acceptance/fio/spool-fsync.fio`
- Create: `tools/hardware-acceptance/fio/rocksdb-random-write.fio`
- Create: `tools/hardware-acceptance/fio/archive-sequential.fio`
- Create: `tools/hardware-acceptance/network.sh`
- Create: `docs/operations/production-topology.md`
- Create: `docs/reviews/hardware-acceptance.md`

**Interfaces:**
- Consumes: observed 30-day event rates, daily archive growth, state/book cardinality, query concurrency, target SLOs, and target host specifications.
- Produces: exact host roles/failure domains, disk/network/CPU/RAM capacity forecasts, acceptance benchmark results, and expansion thresholds.

- [ ] **Step 1: Verify all stage tags and gate artifacts**

```bash
for tag in stage-0-foundations stage-1-truth stage-2-state stage-3-intelligence stage-4-market stage-5-research stage-6-desk; do
  git verify-tag "$tag"
done
```

Expected: every signed stage tag verifies cryptographically and every referenced gate/evidence file exists.

- [ ] **Step 2: Implement the measured capacity model**

Inputs include average/P99/peak event rate, raw/canonical/book bytes per day, Parquet compression, ClickHouse growth, checkpoint size/frequency, NATS retention, replica count, free-space thresholds, research concurrency, and one-year/three-year horizon. Output low/base/high storage, IOPS, bandwidth, CPU, memory, and recovery time forecasts.

- [ ] **Step 3: Freeze host roles and failure domains**

Document primary node/capture, secondary node/capture, two stateful analytics hosts, control/observability host, archive system, and client network. Record provider/rack/power/network independence without storing sensitive addresses in public files.

- [ ] **Step 4: Run hardware acceptance**

`run.sh` verifies CPU features, ECC visibility, NVMe SMART/endurance, mirrored layout, fsync patterns for spool/RocksDB/NATS, ClickHouse merge throughput, archive sequential throughput, 10/25 GbE, clock discipline, thermal behavior, and 24-hour burn-in.

- [ ] **Step 5: Record acceptance and expansion triggers**

Production requires measured headroom for 5x 30-day P99 and 10x average event rate, two simultaneous full-history research scans without hot-path SLO breach, at least 20% archive/spool free space, and recovery within declared RTO. Document purchase/expansion trigger before any resource crosses 70% sustained or forecasted 80% within 90 days.

- [ ] **Step 6: Commit**

```bash
git add infra/ansible/inventory/production infra/ansible/group_vars/production.yml.example infra/capacity tools/capacity-plan tools/hardware-acceptance docs/operations/production-topology.md docs/reviews/hardware-acceptance.md
git commit -m "chore(ops): define production capacity and hardware acceptance"
```

---

### Task 2: Implement network segmentation, WireGuard access, mTLS, and certificate lifecycle

**Files:**
- Create: `infra/ansible/roles/network_zone/defaults/main.yml`
- Create: `infra/ansible/roles/network_zone/tasks/main.yml`
- Create: `infra/ansible/roles/wireguard/defaults/main.yml`
- Create: `infra/ansible/roles/wireguard/tasks/main.yml`
- Create: `infra/ansible/roles/wireguard/templates/wg0.conf.j2`
- Create: `infra/ansible/roles/mtls/defaults/main.yml`
- Create: `infra/ansible/roles/mtls/tasks/main.yml`
- Create: `infra/ansible/roles/mtls/templates/certificate-policy.json.j2`
- Create: `infra/network/policy.yml`
- Create: `infra/network/nftables/acquisition.nft`
- Create: `infra/network/nftables/analytics-core.nft`
- Create: `infra/network/nftables/user-access.nft`
- Create: `infra/network/nftables/execution-empty.nft`
- Create: `tools/network-policy-check/src/main.rs`
- Create: `tools/certificate-check/src/main.rs`
- Create: `docs/security/network-zones.md`
- Create: `docs/runbooks/certificate-rotation.md`
- Create: `docs/runbooks/wireguard-device-revocation.md`

**Interfaces:**
- Consumes: approved trust zones, service identities/ports, desk devices, local CA/identity provider, and production inventory.
- Produces: default-deny firewall rules, private WireGuard access, mutually authenticated service channels, short-lived/rotatable certificates, and machine-verifiable network policy.

- [ ] **Step 1: Write policy tests before firewall templates**

Define allowed flows explicitly:

- Hyperliquid sources -> acquisition adapters as outbound connections.
- Acquisition -> archive/NATS canonical ingress only.
- Core/analytics -> stateful stores and internal gRPC.
- Access/API -> read-only data stores/internal services and identity.
- Desk devices -> API/Kanidm over WireGuard.
- No zone -> future execution zone in V1.
- No public inbound database/NATS/node/admin ports.

`network-policy-check` must reject undeclared flows and any V1 rule targeting an execution/signing service.

- [ ] **Step 2: Implement nftables and interface binding**

Services bind to dedicated private interfaces or loopback; firewall defaults drop input/forward and restrict output per role where practical. Administrative SSH is WireGuard-only with hardware-backed keys and MFA policy.

- [ ] **Step 3: Implement mTLS service identity**

Certificates include stable service/host identity, short validity, allowed usages, and trust-zone constraints. Services verify SAN/issuer/revocation/expiry and map identity to exact permissions. Private keys are root/service-readable only and generated locally.

- [ ] **Step 4: Implement rotation and revocation automation**

Staged rotation overlaps old/new trust, canaries one service, then rolls all. Certificate expiry alerts fire at 30/14/7/2 days. Device loss revokes WireGuard peer, Kanidm session/device, and local client credential.

- [ ] **Step 5: Verify from inside and outside each zone**

```bash
cargo run -p network-policy-check -- verify infra/network/policy.yml
cargo run -p certificate-check -- scan /etc/hyperliquid-alpha-desk/pki
./infra/ansible/tests/check-network.sh
```

Expected: only declared flows succeed; execution-zone probes fail; expired/revoked credentials fail closed.

- [ ] **Step 6: Commit**

```bash
git add infra/ansible/roles/network_zone infra/ansible/roles/wireguard infra/ansible/roles/mtls infra/network tools/network-policy-check tools/certificate-check docs/security/network-zones.md docs/runbooks/certificate-rotation.md docs/runbooks/wireguard-device-revocation.md
git commit -m "feat(security): enforce network zones and mutual authentication"
```

---

### Task 3: Harden hosts, services, secrets, and configuration drift controls

**Files:**
- Create: `infra/ansible/roles/os_hardening/defaults/main.yml`
- Create: `infra/ansible/roles/os_hardening/tasks/main.yml`
- Create: `infra/ansible/roles/service_hardening/defaults/main.yml`
- Create: `infra/ansible/roles/service_hardening/tasks/main.yml`
- Create: `infra/ansible/roles/secret_delivery/defaults/main.yml`
- Create: `infra/ansible/roles/secret_delivery/tasks/main.yml`
- Create: `infra/systemd/hardening/common.conf`
- Create: `infra/systemd/hardening/hl-capture.conf`
- Create: `infra/systemd/hardening/hl-core.conf`
- Create: `infra/systemd/hardening/hl-analytics.conf`
- Create: `infra/systemd/hardening/hl-research.conf`
- Create: `infra/systemd/hardening/hl-api.conf`
- Create: `infra/security/auditd.rules`
- Create: `tools/config-drift/src/main.rs`
- Create: `tools/release-inventory/src/main.rs`
- Create: `docs/security/host-hardening.md`
- Create: `docs/security/secret-management.md`
- Create: `docs/runbooks/config-drift.md`

**Interfaces:**
- Consumes: production inventory, signed release manifest, encrypted secret source, hardened service templates.
- Produces: idempotently hardened Ubuntu hosts, least-privilege services, secure secret materialization, file/config integrity inventory, and drift alerts.

- [ ] **Step 1: Write host-compliance tests**

Check Ubuntu 24.04, staged security updates, Secure Boot where supported, full-disk encryption status, chrony, disabled password/root SSH, minimal packages, audit rules, service users, directory modes, core-dump policy, ptrace restrictions, firewall, and no world-readable secret/config files.

- [ ] **Step 2: Harden each service unit**

Use `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, private temporary directories/devices, capability drop, syscall/address-family restrictions, resource limits, read/write paths, watchdogs, and per-service user/group. Exceptions require an ADR and automated assertion.

- [ ] **Step 3: Implement secret delivery**

Secrets are decrypted/materialized only on target hosts or supplied through protected credentials APIs, with mode 0400/0440 and service ownership. Release/build processes cannot read production secrets. Logs redact configured secret types and tests inject canary values to prove absence.

- [ ] **Step 4: Implement signed configuration and drift detection**

At deploy, hash all binaries, unit files, configs, schemas, model public keys, and dependency image digests. `config-drift` compares host state to signed release inventory and emits high severity on unexplained changes.

- [ ] **Step 5: Verify idempotence and compliance**

```bash
./infra/ansible/tests/check-production-hardening.sh
cargo run -p config-drift -- verify --inventory target/release-inventory.json
```

Expected: second Ansible pass has zero changes and drift report is clean.

- [ ] **Step 6: Commit**

```bash
git add infra/ansible/roles/os_hardening infra/ansible/roles/service_hardening infra/ansible/roles/secret_delivery infra/systemd/hardening infra/security tools/config-drift tools/release-inventory docs/security/host-hardening.md docs/security/secret-management.md docs/runbooks/config-drift.md
git commit -m "feat(security): harden hosts services and secret delivery"
```

---

### Task 4: Complete observability, data-health policy, SLOs, and alert routing

**Files:**
- Create: `infra/monitoring/otel/collector-production.yaml`
- Create: `infra/monitoring/victoriametrics/vmagent.yml`
- Create: `infra/monitoring/victoriametrics/vmalert.yml`
- Create: `infra/monitoring/victoriametrics/scrape.yml`
- Create: `infra/monitoring/dashboards/overview.json`
- Create: `infra/monitoring/dashboards/data-health.json`
- Create: `infra/monitoring/dashboards/storage.json`
- Create: `infra/monitoring/dashboards/research.json`
- Create: `infra/monitoring/dashboards/client.json`
- Create: `infra/monitoring/alerts/data-health.yml`
- Create: `infra/monitoring/alerts/service-slo.yml`
- Create: `infra/monitoring/alerts/storage.yml`
- Create: `infra/monitoring/alerts/security.yml`
- Create: `infra/monitoring/alerts/client.yml`
- Create: `config/health-policy-v1.toml`
- Create: `tools/slo-report/src/main.rs`
- Create: `docs/operations/slo-sli.md`
- Create: `docs/runbooks/alert-routing.md`

**Interfaces:**
- Consumes: all service metrics/logs/traces, health assessments, SLO targets, on-premise alert destinations, and role schedules.
- Produces: local correlated observability, scoped green/amber/red health, burn-rate/SLO reports, severity-based alerts, and operator dashboards.

- [ ] **Step 1: Encode health thresholds exactly**

Implement design defaults for committed source lag, source agreement, canonical state, book, archive/spool space/fsync, feature state, model state, and client state. Health policy is versioned and unit-tested at boundary values.

- [ ] **Step 2: Implement required metrics and trace correlation**

Cover acquisition, state, features/signals, storage/API, and client metrics listed in design section 24.2. Every request/signal/replay/archive manifest links trace/build/schema/model/feature IDs without high-cardinality raw addresses in global labels.

- [ ] **Step 3: Implement SLO computation**

Targets:

- committed observation to spool p99 <25 ms;
- committed block to state p99 <150 ms;
- state delta to features p99 <75 ms;
- feature to deterministic signal p99 <50 ms;
- signal to macOS client p99 <200 ms on healthy LAN/VPN;
- hot API p95 <150 ms and p99 <500 ms;
- standard historical query p95 <3 s;
- zero silent committed loss, replay mismatch, or evidence-less signal;
- capture 99.99% and API 99.9% monthly targets.

- [ ] **Step 4: Implement severity and local routing**

Critical/high/medium/low mapping follows the design. Critical correctness alerts use multiple local channels such as desk display, local gateway, and operator paging system under operator control. Alert deduplication cannot suppress escalation or recovery state.

- [ ] **Step 5: Run synthetic health/SLO tests**

Inject every red/amber condition and assert affected signal scopes, dashboards, API metadata, and client state agree. Generate a 30-day synthetic SLO report and validate burn-rate alerts.

- [ ] **Step 6: Commit**

```bash
git add infra/monitoring config/health-policy-v1.toml tools/slo-report docs/operations/slo-sli.md docs/runbooks/alert-routing.md
git commit -m "feat(ops): operationalize health SLOs and local alerting"
```

---

### Task 5: Implement backup, replication, restore, and disaster-recovery drills

**Files:**
- Create: `infra/backup/archive-replication.sh`
- Create: `infra/backup/postgres-backup.sh`
- Create: `infra/backup/postgres-restore.sh`
- Create: `infra/backup/clickhouse-rebuild.sh`
- Create: `infra/backup/rocksdb-checkpoint-replicate.sh`
- Create: `infra/backup/model-config-backup.sh`
- Create: `tools/restore-drill/src/main.rs`
- Create: `config/backup-policy.toml`
- Create: `docs/operations/backup-recovery.md`
- Create: `docs/runbooks/site-loss.md`
- Create: `docs/reviews/restore-drill.md`

**Interfaces:**
- Consumes: raw/canonical archive, PostgreSQL WAL/base backups, ClickHouse schemas/Parquet, RocksDB checkpoints, model/config/signing public artifacts, off-site operator-controlled storage.
- Produces: encrypted verified replicas, retention policy, restore automation, RPO/RTO evidence, and deterministic post-restore state verification.

- [ ] **Step 1: Define recovery objectives and failure cases**

Record RPO/RTO for capture spool, immutable archive, PostgreSQL control metadata, ClickHouse, RocksDB hot state, models/config, and observability. Include host loss, stateful site loss, archive corruption, ransomware, accidental deletion, and operator error.

- [ ] **Step 2: Implement encrypted replication and backup verification**

Archive copies are content-hash verified and hash-chain checked. PostgreSQL uses continuous WAL archiving plus encrypted full backups. ClickHouse is replicated but rebuildable from Parquet. Recent compatible RocksDB checkpoints replicate independently. Offline model signing private keys remain outside automated online backup; public keys/revocations/configs are backed up.

- [ ] **Step 3: Implement clean-room restore automation**

`restore-drill` provisions a clean isolated environment, restores PostgreSQL, verifies archives, restores a checkpoint, replays to target block, rebuilds ClickHouse from Parquet, loads approved models/config, starts API, and runs known queries/state hashes.

- [ ] **Step 4: Test corruption and site-loss paths**

Corrupt one archive object, remove one manifest, lose the primary site, and restore from secondary/off-site. The process must detect corruption, select a verified generation, and never silently skip missing canonical data.

- [ ] **Step 5: Execute and record quarterly drill**

```bash
cargo run -p restore-drill -- execute config/backup-policy.toml --target isolated
```

Expected: known state/checkpoint/feature/signal/API hashes match, RPO/RTO are recorded, and unresolved deviations block release.

- [ ] **Step 6: Commit**

```bash
git add infra/backup tools/restore-drill config/backup-policy.toml docs/operations/backup-recovery.md docs/runbooks/site-loss.md docs/reviews/restore-drill.md
git commit -m "feat(ops): add verified backup and clean-room recovery"
```

---

### Task 6: Execute load, soak, chaos, and failover qualification

**Files:**
- Create: `tools/load-replay/src/main.rs`
- Create: `tools/soak-runner/src/main.rs`
- Create: `tools/chaos-runner/src/main.rs`
- Create: `tests/performance/scenarios.toml`
- Create: `tests/chaos/scenarios.toml`
- Create: `docs/reviews/load-soak-report.md`
- Create: `docs/reviews/chaos-recovery-report.md`
- Create: `docs/runbooks/failover.md`

**Interfaces:**
- Consumes: production-like recordings, target topology, SLO/health policy, failpoints, recovery procedures.
- Produces: quantified capacity, 24-hour soak evidence, chaos outcomes, failover timing, and verified degraded behavior.

- [ ] **Step 1: Define deterministic qualification scenarios**

Include 5x observed 30-day P99, 10x average, burst replay, 24-hour normal soak with compaction/queries, two simultaneous full-history research scans, reconnect storms, large graph/history queries, and high alert fan-out.

- [ ] **Step 2: Implement fault scenarios**

Kill primary capture, kill core during block commit boundaries, lose one NATS replica, fill a non-critical disk, corrupt spool tail, delay/loss secondary source, stop ClickHouse/PostgreSQL, force book mismatch, expire model/certificate, restore PostgreSQL, rebuild ClickHouse, restore checkpoint, and simulate clock drift.

- [ ] **Step 3: Assert required degraded behavior**

Each scenario has exact expected health, watermark, signal suppression, API/client behavior, recovery command, and final hash. A failure that preserves availability but violates correctness fails qualification.

- [ ] **Step 4: Run soak and chaos on staging topology**

```bash
cargo run -p load-replay -- tests/performance/scenarios.toml
cargo run -p soak-runner -- --duration 24h
cargo run -p chaos-runner -- tests/chaos/scenarios.toml
```

Expected: no silent loss/replay mismatch/evidence-less signal, SLOs within approved bounds, and final deterministic hashes match.

- [ ] **Step 5: Record bottlenecks and repeat after fixes**

Reports include CPU, memory, disk, merge/compaction, queue lag, API/stream latency, recovery times, and all deviations. Release requires a clean final run; prior failed runs remain archived.

- [ ] **Step 6: Commit**

```bash
git add tools/load-replay tools/soak-runner tools/chaos-runner tests/performance tests/chaos docs/reviews/load-soak-report.md docs/reviews/chaos-recovery-report.md docs/runbooks/failover.md
git commit -m "test(ops): qualify load soak chaos and failover"
```

---

### Task 7: Implement supply-chain security, reproducible release, SBOM, and artifact signing

**Files:**
- Create: `.github/workflows/release.yml`
- Create: `tools/release/build.sh`
- Create: `tools/release/sign.sh`
- Create: `tools/release/verify.sh`
- Create: `tools/release/sbom.sh`
- Create: `tools/release/manifest.rs`
- Create: `config/release-policy.toml`
- Create: `docs/security/supply-chain.md`
- Create: `docs/runbooks/release-rollback.md`

**Interfaces:**
- Consumes: clean signed commit/tag, pinned toolchains/dependencies/images, schema/migration compatibility reports, regression benchmarks, signing keys held outside CI.
- Produces: reproducible Rust/Swift artifacts, container/image digests, SBOMs, provenance, signatures, release manifest, and rollback package.

- [ ] **Step 1: Write release-policy tests**

The policy rejects dirty tree, unverified stage tags, unpinned dependency/image, known disallowed advisory/license, generated drift, failed replay/benchmark/migration, missing SBOM/signature, absent rollback artifact, or any V1 `hl-exec` artifact/reference.

- [ ] **Step 2: Build reproducibly**

Set `SOURCE_DATE_EPOCH`, remap paths, use locked dependencies, build on two clean builders, and compare hashes. Swift/macOS artifacts use reproducible settings to the extent supported; signed app bundle provenance records unavoidable signing differences separately from code/resource hashes.

- [ ] **Step 3: Generate SBOM and provenance**

Produce CycloneDX/SPDX for Rust, Swift packages, containers, and infrastructure images. Manifest records source SHA/tag, toolchains, lock hashes, schema fingerprints, migration set, model public-key set, binary/image hashes, test/gate reports, and builders.

- [ ] **Step 4: Sign and verify artifacts offline**

Release signatures are created by approved key holders outside untrusted CI. `verify.sh` checks signatures, hashes, SBOM, provenance, schema compatibility, model public keys, and absence of execution binaries before installation.

- [ ] **Step 5: Package rollback**

Include previous compatible binaries/images/config, migration rollback/forward strategy, model registry state, and exact command. Rollback cannot restore an incompatible state codec without checkpoint/replay validation.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/release.yml tools/release config/release-policy.toml docs/security/supply-chain.md docs/runbooks/release-rollback.md
git commit -m "feat(release): add reproducible signed release pipeline"
```

---

### Task 8: Complete threat modeling, security tests, privacy review, and independent audit

**Files:**
- Create: `docs/security/threat-model-v1.md`
- Create: `docs/security/privacy-attribution-review.md`
- Create: `tests/security/source-payloads/oversized-length.bin`
- Create: `tests/security/source-payloads/deeply-nested.json`
- Create: `tests/security/source-payloads/unknown-enum.json`
- Create: `tests/security/api/authorization-matrix.toml`
- Create: `tests/security/api/query-budget-cases.json`
- Create: `tests/security/api/websocket-abuse.json`
- Create: `tests/security/model-bundles/tampered-manifest.tar`
- Create: `tests/security/model-bundles/schema-mismatch.tar`
- Create: `tests/security/model-bundles/resource-exhaustion.tar`
- Create: `tools/security-regression/run.sh`
- Create: `docs/reviews/security-audit-v1.md`
- Create: `docs/runbooks/security-incident.md`

**Interfaces:**
- Consumes: all trust boundaries, source/parser surfaces, APIs, identity, model loading, archives, clients, and operational controls.
- Produces: reviewed threat model, abuse cases, security regression suite, privacy/attribution language, remediation evidence, and independent release approval.

- [ ] **Step 1: Model assets, actors, boundaries, and abuse cases**

Cover malformed/adversarial payloads, silent loss/reorder, source poisoning/provisional deception, schema drift, dependency/model compromise, insider feature/model approval alteration, credential theft, unauthorized API, research-production leakage, archive destruction/ransomware, stale/misleading UI, and future signer isolation despite its absence.

- [ ] **Step 2: Build security regression fixtures**

Include oversized/deep JSON/MessagePack/Protobuf, malformed decimals, hash collisions/mismatch handling, decompression bombs where applicable, WS frame floods, cursor tampering, authorization bypass, SQL/query abuse, path traversal in artifacts, malicious ONNX/bundle manifests, revoked signatures, cache poisoning, and secret canaries.

- [ ] **Step 3: Verify privacy and attribution behavior**

The product distinguishes protocol hard relations, inferred links, and verified annotations; uses “likely related” language; exposes confidence/evidence/alternatives; avoids claiming real-world ownership; and provides configuration for hiding operator-owned addresses/notes from shared views.

- [ ] **Step 4: Run independent review and remediate findings**

An independent reviewer receives design, threat model, source, release candidate, SBOM, deployment config, and test results. Findings are tracked with severity, evidence, owner, fix commit, retest, and accepted residual risk. Critical/high unresolved findings block release.

- [ ] **Step 5: Verify security release gate**

```bash
./tools/security-regression/run.sh
cargo audit
cargo deny check
```

Expected: all regression tests pass, no disallowed vulnerability/license, audit chain verifies, and `hl-exec` is absent.

- [ ] **Step 6: Commit**

```bash
git add docs/security/threat-model-v1.md docs/security/privacy-attribution-review.md tests/security tools/security-regression docs/reviews/security-audit-v1.md docs/runbooks/security-incident.md
git commit -m "security: complete V1 threat model and independent audit"
```

---

### Task 9: Execute staging canary, production rollout, and rollback rehearsal

**Files:**
- Create: `infra/ansible/playbooks/deploy-staging.yml`
- Create: `infra/ansible/playbooks/deploy-production.yml`
- Create: `infra/ansible/playbooks/rollback.yml`
- Create: `config/canary-policy.toml`
- Create: `tools/canary-check/src/main.rs`
- Create: `docs/operations/release-rollout.md`
- Create: `docs/reviews/staging-canary.md`
- Create: `docs/reviews/production-rollout.md`

**Interfaces:**
- Consumes: signed release bundle, staging/production inventory, source/state regression manifests, SLO/health policy, rollback bundle, role approvals.
- Produces: live staging mirror canary, phased production deployment, source/state/signal/client comparison, and proven rollback.

- [ ] **Step 1: Define canary comparisons and abort conditions**

Compare source continuity/hashes, canonical event counts, state/checkpoint hashes, book health, feature/signal hashes, model results, API schemas, client sequences, latency, and resource usage against current production or approved baseline. Any critical divergence, red correctness state, schema incompatibility, or SLO regression beyond policy aborts.

- [ ] **Step 2: Deploy to staging as a live mainnet read-only mirror**

Run for the configured minimum canary window across normal and stressed activity. Replay approved ranges and compare outputs. No staging write can affect production control/model state.

- [ ] **Step 3: Rehearse rollback before production**

Deploy candidate, generate live state, roll back through `rollback.yml`, catch up from archive/streams, and verify hashes/client resume. Record exact duration and limitations.

- [ ] **Step 4: Roll production in failure-isolated order**

Recommended order: observability/control compatibility, secondary capture, analytics replicas, API canary, primary capture/core with one-at-a-time verification, then client distribution. Never update both independent capture paths simultaneously.

- [ ] **Step 5: Observe post-deploy window and sign rollout record**

Monitor all health/SLO/drift/backup/client metrics through the configured window. Platform, SRE/security, product/desk, and independent reviewer sign the record. Research/risk sign model/signal activation separately.

- [ ] **Step 6: Commit**

```bash
git add infra/ansible/playbooks config/canary-policy.toml tools/canary-check docs/operations/release-rollout.md docs/reviews/staging-canary.md docs/reviews/production-rollout.md
git commit -m "chore(release): add canary rollout and rehearsed rollback"
```

---

### Task 10: Prepare the open-source split and execute the final V1 release gate

**Files:**
- Create: `docs/open-source/repository-split.md`
- Create: `docs/open-source/public-api-boundaries.md`
- Create: `docs/open-source/security-disclosure.md`
- Create: `docs/open-source/contributing.md`
- Create: `docs/open-source/data-and-model-policy.md`
- Create: `tools/open-source-audit/src/main.rs`
- Create: `tools/release-metadata/src/main.rs`
- Create: `config/open-source-policy.toml`
- Create: `config/stage-gates/v1-release.toml`
- Generate after verification: `docs/stage-gates/v1-release.evidence.json`
- Create after approval: `docs/stage-gates/v1-release.md`
- Create: `CHANGELOG.md`
- Modify: `README.md`
- Modify: `LICENSE`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: the production-ready monorepo, public/private classification, license inventory, all signed stage evidence, security/restore/load/canary reports, and the operator-selected public repository URL.
- Produces: a clean public/private export boundary, leak audit, reproducible public package, clean-commit final gate report, signed V1 release commit/tag, and no exposed private alpha or operator data.

- [ ] **Step 1: Define exact public/private export boundaries**

Public candidates are domain types, canonical event framework, spool/archive formats, deterministic reducer interfaces, replay framework, API contracts, generic Swift components, redistributable fixtures, and public documentation. Private material is the operator feed adapter, private historical data, cluster annotations, feature/signal thresholds, model artifacts, promotion results, infrastructure inventory, identities, secrets, and proprietary deployment details.

`config/open-source-policy.toml` lists every top-level path as `public`, `private`, or `generated-review-required`; unclassified paths fail the audit. `docs/open-source/repository-split.md` names the export command and verification checklist.

- [ ] **Step 2: Implement release metadata and the open-source leak audit**

`release-metadata set-repository-url` validates an HTTPS URL ending in `.git` or a canonical HTTPS repository page and writes the exact value into `[workspace.package].repository`; it rejects localhost, credentials, query strings, fragments, and non-HTTPS schemes.

`open-source-audit` scans the full Git history and export tree, binaries, SBOMs, docs, fixtures, configuration, model manifests, and generated artifacts for secrets, private addresses, proprietary source schema content, private alpha thresholds/results, internal hostnames, identity information, restricted data, and disallowed licenses. It verifies public dummy adapter/model plugins compile against the exported interfaces.

Add tests for every classification and leak category, including a seeded canary secret and private signal threshold that must be detected.

- [ ] **Step 3: Write public governance and disclosure documentation**

Document architecture, reproducible build/test, contribution rules, security reporting, data/model licensing, inference limitations, attribution uncertainty, and explicit absence of trading guarantees. The public README states that V1 is read-only and contains no execution or signer capability.

Set the canonical repository URL only when the operator has created the public repository:

```bash
test -n "${PUBLIC_REPOSITORY_URL:?set PUBLIC_REPOSITORY_URL to the canonical HTTPS repository URL}"
cargo run -p release-metadata -- set-repository-url "$PUBLIC_REPOSITORY_URL" Cargo.toml
cargo metadata --format-version 1 --no-deps >/dev/null
```

- [ ] **Step 4: Commit every final-gate input before verification**

```bash
git add docs/open-source tools/open-source-audit tools/release-metadata config/open-source-policy.toml config/stage-gates/v1-release.toml CHANGELOG.md README.md LICENSE Cargo.toml Cargo.lock justfile
git commit -m "chore(release): add V1 release policy and public export controls"
test -z "$(git status --porcelain)"
git rev-parse HEAD
```

The printed SHA is the immutable implementation commit evaluated by the final release gate. Do not amend it.

- [ ] **Step 5: Execute the final gate from fresh clean clones on two builders**

`just v1-release-gate` writes only to ignored `target/stage-gates/v1-release.json` and verifies:

- every signed stage tag and gate record;
- clean reproducible builds on two builders;
- full regression replay and schema/migration dry run;
- final load, soak, and chaos reports;
- clean-room restore evidence;
- threat, security, and privacy audits;
- staging canary and rollback rehearsal;
- signed artifacts, SBOMs, and provenance;
- API and Swift acceptance;
- public export compilation and open-source leak audit;
- route, binary, package, SBOM, and deployment inventories proving no `hl-exec`, signer, or trading credential path.

Run:

```bash
just v1-release-gate
sha256sum target/stage-gates/v1-release.json
```

Expected: PASS and matching canonical comparison hashes on both builders.

- [ ] **Step 6: Commit final evidence, collect approvals, and sign V1**

```bash
cp target/stage-gates/v1-release.json docs/stage-gates/v1-release.evidence.json
cargo run -p stage-gate -- render-record \
  --evidence docs/stage-gates/v1-release.evidence.json \
  --output docs/stage-gates/v1-release.md
git add docs/stage-gates/v1-release.evidence.json docs/stage-gates/v1-release.md
git commit -m "release: approve read-only Hyperliquid Alpha Desk v0.1.0"
git tag -s v0.1.0 -m "Hyperliquid Alpha Desk read-only V1"
git verify-tag v0.1.0
```

Platform, data, security/SRE, product/desk, research/risk, and independent reviewers must sign the approval artifacts referenced by the release record. Do not create the release tag if any required approval, evidence item, public/private classification, or comparison is missing.

## Final V1 Exit Criteria

- Dedicated primary and independent secondary paths pass hardware, network, host, clock, and capacity acceptance.
- Default-deny zones, WireGuard, mTLS, OIDC/passkeys, RBAC, secrets, and drift controls pass independent security review.
- Data health, SLOs, metrics, traces, logs, and alert routing agree across services and clients.
- Clean-room restore reproduces known state, feature, signal, API, and control hashes within approved RPO/RTO.
- Load, soak, chaos, failover, staging canary, and rollback rehearsal pass with no silent correctness failure.
- Release artifacts are reproducible, SBOM-attached, signed, provenance-linked, migration-checked, and reversible.
- Open-source audit finds no private data, proprietary feed implementation, alpha configurations/results, credentials, or internal infrastructure details.
- V1 route, binary, package, SBOM, and deployment inventories contain no execution/signing capability.
- Final gate `docs/stage-gates/v1-release.md` is approved and signed tag `v0.1.0` exists.
