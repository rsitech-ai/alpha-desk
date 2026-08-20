#!/usr/bin/env bash
set -euo pipefail

# ponytail: HLCOV_DOCS_ROOT is a fixture hook (temp tree). Unset in CI; the script directory is the repo.
repo_root="${HLCOV_DOCS_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
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
  grep -Fiq 'signing' "$doc" || fail "$doc does not preserve signing exclusion"
  grep -Fiq 'private key' "$doc" || fail "$doc does not preserve private-key exclusion"
  # Both phrases required. A bare "execution" heading must not satisfy this line.
  grep -Fiq 'order placement' "$doc" || fail "$doc does not preserve order-placement exclusion"
  grep -Ei -q 'copy-trad|copy trading' "$doc" || fail "$doc does not preserve copy-trading exclusion"
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
status_snapshot="$(awk '
  /^## 2026-08-20 snapshot$/ {p=1}
  p && /^## / && $0 != "## 2026-08-20 snapshot" {exit}
  p
' "$STATUS")"
[[ -n "$status_snapshot" ]] || fail "$STATUS missing ## 2026-08-20 snapshot section"
printf '%s\n' "$status_snapshot" | grep -Ei -q 'planned|in progress|in-progress' ||
  fail "$STATUS snapshot section does not mark full-coverage as planned/in-progress"

python3 - "$SPEC" "$PLAN" "$TRACE" "$STATUS" <<'PY'
import re
import sys
import tempfile
from pathlib import Path

spec_path, plan_path, trace_path, status_path = (Path(p) for p in sys.argv[1:5])
neg_re = re.compile(r"\b(?:no|not|never)\b|forbid", re.I)
sentence_split_re = re.compile(r"(?<=[.!?])\s+|\n+")
exclusion_needles = (
    ("/exchange", re.compile(r"/exchange")),
    ("order placement", re.compile(r"order placement", re.I)),
    ("copy trad", re.compile(r"copy[- ]trad", re.I)),
)
inverse_res = (
    re.compile(r"places orders", re.I),
    re.compile(r"place orders", re.I),
    re.compile(r"via\s+`?/exchange", re.I),
    re.compile(r"\b(?:calls?|calling|called|POST)\s+`?/exchange", re.I),
    re.compile(r"\b(?:writes?|writing)\s+(?:to\s+)?`?/exchange", re.I),
    re.compile(r"/exchange writes", re.I),
    re.compile(r"automatic copy[- ]trad", re.I),
    re.compile(r"copy[- ]trad(?:e|ing)\s+(?:is\s+)?enabled", re.I),
    re.compile(r"us(?:e|ing)\s+action signing", re.I),
    re.compile(r"us(?:e|ing)\s+.{0,80}private keys", re.I),
)
usage_near_exchange = re.compile(
    r"\b(?:use|used|uses|using|call|calls|calling|called|write|writes|writing|POST|enable|enabled)\b",
    re.I,
)


def sentences(text: str) -> list[str]:
    return [part for part in sentence_split_re.split(text) if part.strip()]


def doc_errors(label: str, text: str) -> list[str]:
    errors: list[str] = []
    parts = sentences(text)
    for name, needle in exclusion_needles:
        if not any(needle.search(part) and neg_re.search(part) for part in parts):
            errors.append(f"{label} has no no/not/never/forbid exclusion near {name}")
    for inverse in inverse_res:
        for part in parts:
            if neg_re.search(part):
                continue
            match = inverse.search(part)
            if match:
                errors.append(f"{label} inverse claim {match.group(0)!r}")
                break
    for part in parts:
        if "/exchange" not in part or neg_re.search(part):
            continue
        if usage_near_exchange.search(part):
            errors.append(f"{label} treats /exchange as something to call or write")
            break
    return errors


def snapshot_section(status_text: str) -> str:
    lines = status_text.splitlines()
    collected: list[str] = []
    in_section = False
    for line in lines:
        if line == "## 2026-08-20 snapshot":
            in_section = True
            collected.append(line)
            continue
        if in_section and line.startswith("## ") and line != "## 2026-08-20 snapshot":
            break
        if in_section:
            collected.append(line)
    return "\n".join(collected)


def snapshot_errors(label: str, text: str) -> list[str]:
    errors: list[str] = []
    for part in sentences(text):
        if neg_re.search(part):
            continue
        gate_passed = re.search(r"\bgate\s+(?:had\s+|has\s+)?passed\b", part, re.I)
        full_passed = re.search(r"full[- ]coverage", part, re.I) and re.search(
            r"\bpassed\b", part, re.I
        )
        if gate_passed or full_passed:
            errors.append(f"{label} claims the full-coverage gate PASSED")
            break
    return errors


def fail_if(condition: bool, message: str, errors: list[str]) -> None:
    if condition:
        errors.append(message)


spec_text = spec_path.read_text()
plan_text = plan_path.read_text()
trace_text = trace_path.read_text()
status_text = status_path.read_text()
snap_text = snapshot_section(status_text)
errors = (
    doc_errors(str(spec_path), spec_text)
    + doc_errors(str(plan_path), plan_text)
    + doc_errors(str(trace_path), trace_text)
    + snapshot_errors(str(status_path), snap_text)
)

good_s11 = (
    "### 1.1 Non-negotiable constraints\n"
    "- No `/exchange`, signing, execution, credentials, private keys, "
    "order placement, or copy trading.\n"
)
good_snap = (
    "## 2026-08-20 snapshot\n"
    "STATUS is a snapshot. Runtime maturity differs by component. Capture and some "
    "Stage 2 replay paths are implemented locally. Wallet, analytics, API, and desk "
    "surfaces remain scaffold or planned. The Hyperliquid full-coverage expansion is "
    "planned and in progress on this branch. It is not a passed gate.\n"
)
bad_plan = (
    "The project now places orders via `/exchange` using action signing and private keys "
    "with automatic copy trading enabled. Order placement is a product capability.\n"
)
bad_ambiguity = (
    "There is no ambiguity here. The desk places orders via `/exchange` for tracked "
    "wallets, and automatic copy trading is enabled by default for tier-1 accounts.\n"
)
good_trace = (
    "Read-only scope is unchanged: no `/exchange`, signing, private keys, "
    "order placement, or copy-trading execution.\n"
)
bad_trace = (
    "Read-only scope is lifted: the desk places orders via /exchange and "
    "automatic copy trading is enabled.\n"
)
bad_snap = (
    "## 2026-08-20 snapshot\n"
    "Wallet, analytics, API, and desk surfaces remain scaffold or planned. "
    "The Hyperliquid full-coverage expansion gate PASSED.\n"
)
bad_snap_lower = (
    "## 2026-08-20 snapshot\n"
    "Wallet, analytics, API, and desk surfaces remain scaffold or planned. "
    "The full-coverage expansion gate passed.\n"
)

with tempfile.TemporaryDirectory() as tmp:
    tmp_path = Path(tmp)
    cases = (
        ("inverse-plan.md", bad_plan, "doc", True),
        ("plan-s11.md", good_s11, "doc", False),
        ("no-ambiguity.md", bad_ambiguity, "doc", True),
        ("trace-ok.md", good_trace, "doc", False),
        ("trace-inverse.md", bad_trace, "doc", True),
        ("status-passed.md", bad_snap, "status", True),
        ("status-passed-lower.md", bad_snap_lower, "status", True),
        ("status-snapshot.md", good_snap, "status", False),
    )
    for name, body, kind, must_fail in cases:
        path = tmp_path / name
        path.write_text(body)
        text = path.read_text()
        found = (
            snapshot_errors(str(path), text)
            if kind == "status"
            else doc_errors(str(path), text)
        )
        if must_fail:
            fail_if(not found, f"polarity self-check: {name} must fail", errors)
        else:
            fail_if(bool(found), f"polarity self-check: {name} must pass: {found}", errors)

if errors:
    print("hlcov-docs:error " + errors[0], file=sys.stderr)
    for extra in errors[1:]:
        print("hlcov-docs:error " + extra, file=sys.stderr)
    raise SystemExit(1)
PY

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
