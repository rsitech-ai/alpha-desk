#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 0 ]]; then
  printf 'FAIL linux-verifier:arguments-forbidden\n' >&2
  exit 64
fi

readonly REPO_ROOT=/workspace
readonly UNIT="$REPO_ROOT/infra/systemd/hl-service@.service"
readonly QUADLETS="$REPO_ROOT/infra/podman/quadlet"
readonly GENERATOR=/usr/lib/systemd/system-generators/podman-system-generator

fail() {
  printf 'FAIL %s\n' "$1" >&2
  exit 1
}

systemd_version="$(
  systemd-analyze --version |
    awk 'NR == 1 && $1 == "systemd" && $2 ~ /^[0-9]+$/ { print $2 }'
)"
[[ "$systemd_version" == 255 ]] || fail "systemd-version:expected-255"
printf 'PASS systemd-version:255\n'

podman_version="$(podman --version | awk '{print $3}')"
[[ "$podman_version" == 4.9.3 ]] || fail "podman-version:expected-4.9.3"
printf 'PASS podman-version:4.9.3\n'

tmp_root="$(mktemp -d /tmp/alpha-task9-linux.XXXXXX)"
readonly tmp_root
trap 'rm -rf -- "$tmp_root"' EXIT

mkdir -p /opt/hyperliquid-alpha-desk/bin
printf '#!/bin/sh\nexit 0\n' >/opt/hyperliquid-alpha-desk/bin/i
printf '#!/bin/sh\nexit 0\n' >/opt/hyperliquid-alpha-desk/bin/hl-api
chmod 0755 \
  /opt/hyperliquid-alpha-desk/bin/i \
  /opt/hyperliquid-alpha-desk/bin/hl-api

materialized="$tmp_root/hl-service@hl-api.service"
sed 's/%i/hl-api/g' "$UNIT" >"$materialized"

systemd-analyze verify --recursive-errors=yes "$UNIT" "$materialized" ||
  fail "systemd-verify"
printf 'PASS systemd-verify:template-and-hl-api\n'

systemd-analyze security --offline=yes "$materialized" >"$tmp_root/security.txt" ||
  fail "systemd-security-offline"
printf 'PASS systemd-security-offline:advisory-generated\n'

[[ -x "$GENERATOR" ]] || fail "quadlet-generator:missing"
mkdir -p "$tmp_root/runtime"
XDG_RUNTIME_DIR="$tmp_root/runtime" \
  QUADLET_UNIT_DIRS="$QUADLETS" \
  "$GENERATOR" --user --dryrun >"$tmp_root/quadlet.txt" ||
  fail "quadlet-generator"

for name in nats postgresql clickhouse minio; do
  grep -F "$name" "$tmp_root/quadlet.txt" >/dev/null ||
    fail "quadlet-generator:missing-$name"
done
printf 'PASS quadlet-generator:podman-4.9.3-dryrun\n'
