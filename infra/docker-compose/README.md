# Local data infrastructure

This Compose project is a reproducible **local-development-only** dependency
stack. It uses the fixed project name `alpha-desk-dev`, runs on an
outbound-capable local bridge whose default host binding is `127.0.0.1`, binds
every published port explicitly to `127.0.0.1`, stores data in project-specific
named volumes, and contains only explicit non-secret development credentials.

It is not a production deployment, production security claim, or durable
observability system. The local bridge is not a network-isolation boundary.
Traces and logs go to the OpenTelemetry debug exporter. The resource and PID
limits are laptop guardrails that still require realistic ingest testing.

## Provisional contracts

- `alpha-archive` is the provisional local MinIO bucket. A later archive
  contract must freeze or replace this name before application code depends on
  it.
- `hl.v1.deadletter.>` is the provisional dead-letter subject family. A later
  NATS contract must freeze or replace it.

## MinIO acceptance

Task 7 accepts the digest-pinned legacy MinIO server and client images for
isolated local development only. Both upstream repositories were archived on
2026-04-25, the binaries are unmaintained, and the images are AGPL-3.0. This is
an explicit short-term maintenance and licensing acceptance, not approval for
production distribution. A maintained S3-compatible replacement or reviewed
source build remains the stronger long-term path.

## Lifecycle

`just dev-up` requires the Docker CLI, `curl`, `jq`, `pg_isready`, and `psql`.
It starts the stack and waits for NATS JetStream, ClickHouse, PostgreSQL,
MinIO, OpenTelemetry, and VictoriaMetrics. NATS and MinIO are reported ready
only after their fixed-project initializer resolves to exactly one container
whose final state is `exited` with exit code zero. `just dev-down` stops the
stack without deleting data.

`just dev-reset` is intentionally destructive: it removes all five
`alpha-desk-dev` named data volumes. The recipe prints this boundary before it
runs. It does not target containers or volumes from other Compose projects.
