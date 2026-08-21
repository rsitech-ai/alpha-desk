#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repository_root"

cargo +1.97.1 test -p hl-capture --test info_rest --locked --offline

if [[ "${HL_INFO_CAPTURE_E2E:-}" == "1" ]]; then
  cargo +1.97.1 test -p hl-capture --test info_rest --locked -- --ignored live_official_all_mids
fi
