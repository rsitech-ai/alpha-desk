#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
gate="$repo_root/tools/ci/check-hyperliquid-full-coverage-docs.sh"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/alpha-desk-hlcov-docs.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT

copy_docs() {
  local dest="$1"
  mkdir -p \
    "$dest/docs/superpowers/specs" \
    "$dest/docs/superpowers/plans"
  cp "$repo_root/docs/superpowers/specs/2026-08-19-hyperliquid-full-coverage-expansion.md" \
    "$dest/docs/superpowers/specs/"
  cp "$repo_root/docs/superpowers/plans/2026-08-19-hyperliquid-full-coverage-plan.md" \
    "$dest/docs/superpowers/plans/"
  cp "$repo_root/docs/superpowers/plans/2026-08-19-hyperliquid-full-coverage-traceability.md" \
    "$dest/docs/superpowers/plans/"
  cp "$repo_root/docs/superpowers/plans/README.md" \
    "$dest/docs/superpowers/plans/"
  cp "$repo_root/docs/ROADMAP.md" "$dest/docs/"
  cp "$repo_root/docs/STATUS.md" "$dest/docs/"
}

run_gate() {
  local root="$1"
  HLCOV_DOCS_ROOT="$root" "$gate"
}

expect_fail() {
  local root="$1"
  local needle="$2"
  local stderr_file="$3"
  local label="$4"
  set +e
  run_gate "$root" >"$stderr_file.stdout" 2>"$stderr_file"
  local status=$?
  set -e
  if ((status == 0)); then
    echo "$label must fail the docs gate" >&2
    exit 1
  fi
  if ! grep -Fq "$needle" "$stderr_file"; then
    cat "$stderr_file" >&2
    echo "$label must report: $needle" >&2
    exit 1
  fi
}

good_root="$fixture_root/good"
copy_docs "$good_root"
if ! run_gate "$good_root"; then
  echo "a copy of the real spec/plan/STATUS must pass the docs gate" >&2
  exit 1
fi

inverse_root="$fixture_root/inverse-plan"
copy_docs "$inverse_root"
cat >"$inverse_root/docs/superpowers/plans/2026-08-19-hyperliquid-full-coverage-plan.md" <<'EOF'
# Inverse trading plan

**Design dependency:** `docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md`

The project now places orders via `/exchange` using action signing and private keys
with automatic copy trading enabled. Order placement is a product capability.
EOF
expect_fail "$inverse_root" "inverse claim" "$fixture_root/inverse-plan.err" \
  "reviewer inverse plan"

passed_root="$fixture_root/status-passed"
copy_docs "$passed_root"
python3 - "$passed_root/docs/STATUS.md" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text()
old = (
    "The Hyperliquid full-coverage expansion is planned and in progress on this branch. "
    "It is not a passed gate."
)
new = "The Hyperliquid full-coverage expansion gate PASSED."
if old not in text:
    raise SystemExit("STATUS snapshot sentence missing; rewrite would not prove polarity")
path.write_text(text.replace(old, new, 1))
PY
expect_fail "$passed_root" "gate PASSED" "$fixture_root/status-passed.err" \
  "STATUS snapshot rewritten to gate PASSED"

printf 'hlcov-docs-polarity-test:ok\n'
