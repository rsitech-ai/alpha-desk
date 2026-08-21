#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repository_root"

cargo +1.97.1 test -p hl-capture --test provisional_pipeline --test public_ws --locked --offline
cargo +1.97.1 test -p hl-core --test input --locked --offline
