#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

readonly SPEC='docs/superpowers/specs/2026-08-19-hyperliquid-full-coverage-expansion.md'
readonly PLAN='docs/superpowers/plans/2026-08-19-hyperliquid-full-coverage-plan.md'
readonly TRACE='docs/superpowers/plans/2026-08-19-hyperliquid-full-coverage-traceability.md'
readonly PLANS_INDEX='docs/superpowers/plans/README.md'
readonly ROADMAP='docs/ROADMAP.md'
readonly STATUS='docs/STATUS.md'
readonly BASE_DESIGN='docs/superpowers/specs/2026-07-24-hyperliquid-alpha-desk-design.md'
readonly STALE_SPEC='docs/superpowers/specs/2026-08-19-hyperliquid-full-coverage-expansion-spec.md'
readonly STALE_PLAN='docs/superpowers/plans/2026-08-19-hyperliquid-full-coverage-implementation-plan.md'

fail() {
  printf 'hlcov-docs:error %s\n' "$1" >&2
  exit 1
}

[[ ! -e "$STALE_SPEC" ]] || fail "stale copy remains: $STALE_SPEC"
[[ ! -e "$STALE_PLAN" ]] || fail "stale copy remains: $STALE_PLAN"

for path in "$SPEC" "$PLAN" "$TRACE" "$PLANS_INDEX" "$ROADMAP" "$STATUS"; do
  [[ -f "$path" ]] || fail "missing:$path"
done

for doc in "$SPEC" "$PLAN"; do
  grep -Fq '/exchange' "$doc" || fail "$doc does not preserve /exchange exclusion"
  grep -Ei -q 'sign(ing|er)|action signing' "$doc" || fail "$doc does not preserve signing exclusion"
  grep -Fiq 'private key' "$doc" || fail "$doc does not preserve private-key exclusion"
  grep -Ei -q 'order placement|copy-trad|execution' "$doc" ||
    fail "$doc does not preserve order-placement/copy-trading/execution exclusion"
done

grep -Fq 'design-approved-v1.0.0' "$SPEC" || fail "$SPEC does not cite tag design-approved-v1.0.0"
grep -Fq 'spec-v1.0.0' "$SPEC" || fail "$SPEC does not cite tag spec-v1.0.0"

if [[ -f "$BASE_DESIGN" ]]; then
  for doc in "$SPEC" "$PLAN" "$PLANS_INDEX" "$ROADMAP"; do
    grep -Fq "$BASE_DESIGN" "$doc" || fail "$doc does not reference $BASE_DESIGN"
  done
else
  for doc in "$SPEC" "$PLAN"; do
    if ! grep -Fq "$BASE_DESIGN" "$doc" &&
      ! grep -Fq 'design-approved-v1.0.0' "$doc"; then
      fail "$doc does not reference the approved base design path or tag design-approved-v1.0.0"
    fi
  done
  for doc in "$PLANS_INDEX" "$ROADMAP"; do
    grep -Fq "$BASE_DESIGN" "$doc" || fail "$doc does not cite the V1 design path"
    grep -Fq 'design-approved-v1.0.0' "$doc" || fail "$doc does not cite tag design-approved-v1.0.0"
    grep -Fq 'spec-v1.0.0' "$doc" || fail "$doc does not cite tag spec-v1.0.0"
    grep -Ei -q 'absent|not in this tree|not present' "$doc" ||
      fail "$doc does not say the V1 design file is absent on this branch"
  done
fi

# ponytail: pins the T01 snapshot date. Upgrade: accept any `## YYYY-MM-DD snapshot` heading that still calls full-coverage planned/in-progress.
grep -Fq '2026-08-20' "$STATUS" || fail "$STATUS missing 2026-08-20 snapshot date"
grep -Fiq 'snapshot' "$STATUS" || fail "$STATUS missing snapshot language"
grep -Ei -q 'full[- ]coverage' "$STATUS" || fail "$STATUS does not mention the full-coverage expansion"
grep -Ei -q 'planned|in progress|in-progress' "$STATUS" ||
  fail "$STATUS does not mark full-coverage as planned/in-progress"

if ! id_count="$(
  python3 - "$SPEC" "$TRACE" <<'PY'
import re
import sys
from pathlib import Path

spec_path = Path(sys.argv[1])
trace_path = Path(sys.argv[2])
id_re = re.compile(r"HLCOV-(?:SRC|PROTO|CORE|WALLET|EVM|ANALYTICS|API|OPS)-[0-9]{3}")
task_re = re.compile(r"\bT(?:0[1-9]|[1-3][0-9]|40)\b")
check_re = re.compile(
    r"just |cargo |pytest |swift test|\btests?\b|/test|e2e|check|replay|scan|gate|fixture",
    re.IGNORECASE,
)

spec_ids = list(dict.fromkeys(id_re.findall(spec_path.read_text())))
if not spec_ids:
    print("hlcov-docs:error addendum has no HLCOV-* requirement IDs", file=sys.stderr)
    raise SystemExit(1)

required_prefixes = (
    "HLCOV-SRC-",
    "HLCOV-PROTO-",
    "HLCOV-CORE-",
    "HLCOV-WALLET-",
    "HLCOV-EVM-",
    "HLCOV-ANALYTICS-",
    "HLCOV-API-",
    "HLCOV-OPS-",
)
for prefix in required_prefixes:
    if not any(req_id.startswith(prefix) for req_id in spec_ids):
        print(f"hlcov-docs:error addendum missing requirement prefix {prefix}*", file=sys.stderr)
        raise SystemExit(1)

trace_text = trace_path.read_text()
missing = [req_id for req_id in spec_ids if req_id not in trace_text]
if missing:
    preview = ", ".join(missing[:8])
    print(
        f"hlcov-docs:error traceability missing {len(missing)} spec IDs, first: {preview}",
        file=sys.stderr,
    )
    raise SystemExit(1)

unwired = []
for req_id in spec_ids:
    row = None
    for line in trace_text.splitlines():
        if req_id in line and line.lstrip().startswith("|"):
            row = line
            break
    if row is None:
        unwired.append(f"{req_id} (no table row)")
        continue
    cells = [cell.strip() for cell in row.strip().strip("|").split("|")]
    if len(cells) < 5:
        unwired.append(f"{req_id} (need task, test/check, evidence columns)")
        continue
    row_body = "|".join(cells[1:])
    if not task_re.search(row_body):
        unwired.append(f"{req_id} (no T01-T40 task)")
        continue
    if not check_re.search(row_body):
        unwired.append(f"{req_id} (no target test/check)")
        continue
    if not cells[-1]:
        unwired.append(f"{req_id} (empty acceptance evidence)")
if unwired:
    preview = "; ".join(unwired[:8])
    print(
        f"hlcov-docs:error traceability unwired {len(unwired)} IDs, first: {preview}",
        file=sys.stderr,
    )
    raise SystemExit(1)

print(len(spec_ids))
PY
)"; then
  exit 1
fi

printf 'hlcov-docs:ok spec=%s plan=%s ids=%s\n' "$SPEC" "$PLAN" "$id_count"
