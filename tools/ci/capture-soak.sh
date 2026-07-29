#!/usr/bin/env bash
set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
readonly repository_root
readonly duration="${DURATION:-10m}"

case "$duration" in
  *s) duration_value="${duration%s}"; duration_multiplier=1 ;;
  *m) duration_value="${duration%m}"; duration_multiplier=60 ;;
  *h) duration_value="${duration%h}"; duration_multiplier=3600 ;;
  *) duration_value="$duration"; duration_multiplier=1 ;;
esac

[[ "$duration_value" =~ ^[0-9]+$ ]] || {
  printf '%s\n' 'capture-soak:error DURATION must be 10s..24h using s, m, h, or integer seconds' >&2
  exit 2
}
duration_seconds="$((10#$duration_value * duration_multiplier))"
[[ "$duration_seconds" =~ ^[0-9]+$ ]] \
  && ((duration_seconds >= 10 && duration_seconds <= 86400)) || {
    printf '%s\n' 'capture-soak:error DURATION must be 10s..24h using s, m, h, or integer seconds' >&2
    exit 2
  }

readonly block_delay_millis="${CAPTURE_SOAK_BLOCK_DELAY_MILLIS:-1000}"
[[ "$block_delay_millis" =~ ^[0-9]+$ ]] \
  && ((block_delay_millis >= 1 && block_delay_millis <= 60000)) || {
    printf '%s\n' 'capture-soak:error CAPTURE_SOAK_BLOCK_DELAY_MILLIS must be 1..60000' >&2
    exit 2
  }
block_count="$(((duration_seconds * 1000 + block_delay_millis - 1) / block_delay_millis))"
((block_count <= 10000000)) || {
  printf '%s\n' 'capture-soak:error requested duration exceeds the fixture block bound' >&2
  exit 2
}

CAPTURE_E2E_BLOCKS="$block_count" \
CAPTURE_E2E_BLOCK_DELAY_MILLIS="$block_delay_millis" \
CAPTURE_E2E_MIN_RUNTIME_SECONDS="$duration_seconds" \
  "${repository_root}/tools/ci/capture-e2e.sh"
