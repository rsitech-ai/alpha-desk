#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repository_root"

cargo +1.97.1 test -p hl-capture --test subscription_plan --test public_ws --locked --offline

if [[ "${HL_PUBLIC_WS_E2E:-}" == "1" ]]; then
  cargo +1.97.1 test -p hl-capture --test public_ws --locked -- --ignored live_official_all_mids
fi
