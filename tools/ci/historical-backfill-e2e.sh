#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repository_root"

cargo +1.97.1 test -p storage-ports --locked --offline
cargo +1.97.1 test -p hl-capture --test historical_s3 --locked --offline
cargo +1.97.1 test -p hl-capture --test config --locked --offline historical_s3
