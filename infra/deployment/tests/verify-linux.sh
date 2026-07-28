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
readonly ALLOWED_INSTANCES=(
  hl-analytics
  hl-api
  hl-capture
  hl-core
  hl-research
)

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
mkdir -p /usr/libexec/hyperliquid-alpha-desk
printf '#!/bin/sh\nexit 0\n' >/opt/hyperliquid-alpha-desk/bin/i
install -m 0755 \
  "$REPO_ROOT/infra/systemd/validate-instance.sh" \
  /usr/libexec/hyperliquid-alpha-desk/validate-instance
chmod 0755 /opt/hyperliquid-alpha-desk/bin/i

materialized_units=("$UNIT")
for instance in "${ALLOWED_INSTANCES[@]}"; do
  printf '#!/bin/sh\nexit 0\n' \
    >"/opt/hyperliquid-alpha-desk/bin/$instance"
  chmod 0755 "/opt/hyperliquid-alpha-desk/bin/$instance"
  materialized="$tmp_root/hl-service@$instance.service"
  sed "s/%i/$instance/g" "$UNIT" >"$materialized"
  materialized_units[${#materialized_units[@]}]="$materialized"
done
printf '#!/bin/sh\nexit 0\n' >/opt/hyperliquid-alpha-desk/bin/hl-exec
chmod 0755 /opt/hyperliquid-alpha-desk/bin/hl-exec
forbidden_materialized="$tmp_root/hl-service@hl-exec.service"
sed 's/%i/hl-exec/g' "$UNIT" >"$forbidden_materialized"
materialized_units[${#materialized_units[@]}]="$forbidden_materialized"

systemd-analyze verify \
  --recursive-errors=yes \
  "${materialized_units[@]}" ||
  fail "systemd-verify"
printf 'PASS systemd-verify:template-five-allowed-and-forbidden\n'

for instance in "${ALLOWED_INSTANCES[@]}"; do
  /usr/libexec/hyperliquid-alpha-desk/validate-instance "$instance" >/dev/null ||
    fail "systemd-validator:allowed-$instance"
done
if /usr/libexec/hyperliquid-alpha-desk/validate-instance hl-exec >/dev/null 2>&1; then
  fail "systemd-validator:forbidden-instance-accepted"
fi
printf 'PASS systemd-validator:five-allowed-and-materialized-hl-exec-rejected\n'

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
