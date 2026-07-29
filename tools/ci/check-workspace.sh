#!/usr/bin/env bash
set -euo pipefail

required=(
  Cargo.toml rust-toolchain.toml justfile
  crates/domain-types/Cargo.toml
  crates/canonical-events/Cargo.toml
  crates/telemetry/Cargo.toml
  services/hl-capture/Cargo.toml
  services/hl-core/Cargo.toml
  services/hl-analytics/Cargo.toml
  services/hl-research/Cargo.toml
  services/hl-api/Cargo.toml
  tools/spool-inspect/Cargo.toml
  tools/canonical-inspect/Cargo.toml
  fuzz/Cargo.toml
  apps/AlphaDesk/Package.swift
)

for path in "${required[@]}"; do
  [[ -f "$path" ]] || { echo "missing:$path" >&2; exit 1; }
done

cargo metadata --format-version 1 --no-deps >/dev/null
swift package --package-path apps/AlphaDesk describe >/dev/null
printf 'workspace-layout:ok\n'
