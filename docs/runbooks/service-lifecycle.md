# Alpha Desk service lifecycle — Stage 0 inactive scaffold

The only allowed V1 instances are:

```text
hl-capture
hl-core
hl-analytics
hl-research
hl-api
```

`hl-exec`, signer, order-router, risk-engine, and every other execution-like or
unknown instance are forbidden. The future-execution zone is empty. Stage 0
contains no signer, trading key, order path, executable deployment, production
inventory, enabled service, or running dependency.

## Scope and status

This runbook documents an inactive Ubuntu 24.04 scaffold. Passing repository
checks proves syntax and declared policy only. It does not prove package
behavior, systemd sandbox runtime, rootless Podman lifecycle, network policy,
secret resolution, backup/restore, or production readiness.

The example inventory uses only `.invalid` names and is never an operational
target. Before any later operation, an approved environment plan must name the
host, owner, change window, rollback, evidence path, and exact allowed service.
Validate the service name with `infra/systemd/validate-instance.sh`.
The installed unit repeats that gate through
`ExecCondition=/usr/libexec/hyperliquid-alpha-desk/validate-instance %i`;
unknown or execution-like instances cannot reach `ExecStart`.

## Stage 0 read-only inspection

These commands observe files or state; they do not authorize a deployment:

```sh
systemctl status 'hl-service@hl-api.service'
journalctl --unit 'hl-service@hl-api.service' --since '-15 minutes'
systemd-analyze verify --recursive-errors=yes hl-service@.service
sshd -t
chronyd -p
```

An absent unit, user, binary, credential, or runtime is expected in Stage 0.
Do not turn an observation into an activation attempt.

## Foundation target down

Confirm the scrape target and job labels, distinguish missing telemetry from an
explicit `up=0`, preserve logs, and escalate to the SRE/security owner. The
alert proves reachability failure only; it does not identify application cause.

## Telemetry series missing

Check scrape discovery, the OpenTelemetry collector, and metric exposition
without inferring that an absent series means GREEN or RED. The current
`alpha_desk_health_assessments_total` metric is cumulative.

## Recent RED assessment

An increment of
`alpha_desk_health_assessments_total{state="red"}` means a RED assessment was
recorded recently. It is not a current-state gauge. Preserve all emitted
labels and the relevant evidence bundle; do not claim the system remains RED
without a separate current-state contract.

## Mutating lifecycle procedures

All start, stop, restart, enable, disable, daemon-reload, user-linger,
firewall, secret, package-upgrade, and restore operations are **later operator
procedures — do not execute in Stage 0**.

A later approved procedure must:

1. name an allowed instance and validate it before constructing a unit name;
2. prove the user/group, writable directories, non-secret configuration,
   separately delivered binary, and secret references exist;
3. retain a verified access session before SSH or firewall changes;
4. record pre-change status and a rollback that does not discard data;
5. start only in a disposable Ubuntu 24.04 VM first;
6. verify graceful stop, restart policy, logs, resource limits, and every
   systemd sandbox directive under measured workload;
7. verify rootless Podman 4.9.3, cgroup v2, UID mappings, persistent-volume
   ownership, loopback reachability, secret resolution, and reboot behavior;
8. require separate authorization for production.

## Residual runtime gates

The following remain BLOCKED on a disposable Ubuntu 24.04 VM or later isolated
restore drill:

- package resolution/upgrades and OpenSSH socket-activation rollback;
- refresh of the exact reviewed Noble package versions and APT pin policy;
- key access plus denied root/password login;
- external default-deny firewall testing and exact zone flows;
- chrony sources, offset, failover, and reboot persistence;
- systemd 255 service startup/shutdown with real `hl-*` binaries;
- rootless Quadlet generator placement, user manager, cgroup v2, linger,
  persistent volumes, and reboot behavior;
- registry/mirror availability for the pinned image identities;
- real scrape labels, alert delivery, and a current-health gauge;
- PostgreSQL PITR, NATS recovery, version-aware object restore, ClickHouse
  rebuild from Parquet, and deterministic checkpoint equality.
