#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repository_root"

cargo +1.97.1 test -p orderbook --locked --offline
cargo +1.97.1 test -p book-inspect --locked --offline
cargo +1.97.1 run -p book-inspect --locked --offline -- replay fixtures/golden/books/snapshot-diffs.json
cargo +1.97.1 run -p book-inspect --locked --offline -- replay fixtures/golden/books/fifo-trigger.json
cargo +1.97.1 test -p canonical-ledger --test order_state --locked --offline -- rested_order_projects
cargo +1.97.1 test -p canonical-ledger --test market_state --locked --offline -- exact_market_creation
cargo +1.97.1 test -p canonical-ledger --test composite_account_state --locked --offline -- project_l4_book
