#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(
  CDPATH='' builtin cd -- "$(command dirname -- "${BASH_SOURCE[0]}")" &&
    builtin pwd -P
)"
readonly SCRIPT_DIR
REPO_ROOT="$(
  CDPATH='' builtin cd -- "$SCRIPT_DIR/../.." &&
    builtin pwd -P
)"
readonly REPO_ROOT
readonly FIXED_EPOCH="${SOURCE_DATE_EPOCH:-1784894400}"

case "$FIXED_EPOCH" in
  ''|*[!0-9]*)
    printf 'generated-check:error SOURCE_DATE_EPOCH must be an unsigned integer\n' >&2
    exit 2
    ;;
esac

for tool in awk basename cargo cmp diff dirname find git jq mkdir mktemp rm rustc shasum tail tr xxd; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'generated-check:error required tool is unavailable: %s\n' "$tool" >&2
    exit 2
  fi
done

if [[ -n "$(git -C "$REPO_ROOT" status --porcelain=v1 --untracked-files=all)" ]]; then
  printf 'generated-check:error repository must be clean so committed generation inputs can be checked\n' >&2
  exit 2
fi

TEMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/alpha-desk-generated.XXXXXX")"
readonly TEMP_ROOT
readonly CHECKOUT="$TEMP_ROOT/checkout"
worktree_added=false

cleanup() {
  local status=$?
  if [[ "$worktree_added" == true ]]; then
    git -C "$REPO_ROOT" worktree remove --force "$CHECKOUT" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$TEMP_ROOT"
  exit "$status"
}
trap cleanup EXIT INT TERM

git -C "$REPO_ROOT" worktree add --detach "$CHECKOUT" HEAD >/dev/null
worktree_added=true
cd -- "$CHECKOUT"

readonly TARGET_FIXTURES="$TEMP_ROOT/target-fixtures"
readonly TARGET_MATERIAL="$TEMP_ROOT/target-material"
readonly TARGET_CONTRACT_A="$TEMP_ROOT/target-contract-a"
readonly TARGET_CONTRACT_B="$TEMP_ROOT/target-contract-b"
readonly TARGET_COMPAT="$TEMP_ROOT/target-compat"
readonly TARGET_BUILD_A="$TEMP_ROOT/target-build-a"
readonly TARGET_BUILD_B="$TEMP_ROOT/target-build-b"
readonly CONTRACT_A="$TEMP_ROOT/contracts-a"
readonly CONTRACT_B="$TEMP_ROOT/contracts-b"
mkdir -p "$CONTRACT_A/rust" "$CONTRACT_B/rust"

CARGO_TARGET_DIR="$TARGET_FIXTURES" \
  cargo +1.97.1 run -p fixture-inspect --frozen --offline -- \
  generate-manifest --root fixtures/golden
git diff --exit-code -- fixtures/golden/manifest.toml

CARGO_TARGET_DIR="$TARGET_MATERIAL" \
  cargo +1.97.1 run -p api-contracts --bin schema-generate --frozen --offline -- \
  material --schema-root schemas/proto --output "$TEMP_ROOT/current.material"
cmp crates/telemetry/schema-fingerprint-v1.material "$TEMP_ROOT/current.material"

CARGO_TARGET_DIR="$TARGET_CONTRACT_A" \
  cargo +1.97.1 run -p api-contracts --bin schema-generate --frozen --offline -- \
  contracts --descriptor "$CONTRACT_A/current.pb" --rust-out "$CONTRACT_A/rust"
CARGO_TARGET_DIR="$TARGET_CONTRACT_B" \
  cargo +1.97.1 run -p api-contracts --bin schema-generate --frozen --offline -- \
  contracts --descriptor "$CONTRACT_B/current.pb" --rust-out "$CONTRACT_B/rust"
cmp "$CONTRACT_A/current.pb" "$CONTRACT_B/current.pb"
diff -ru "$CONTRACT_A/rust" "$CONTRACT_B/rust"

find "$CONTRACT_A/rust" -maxdepth 1 -type f -name '*.rs' -exec basename '{}' \; |
  sort >"$TEMP_ROOT/generated-rust-names.txt"
if ! diff -u - "$TEMP_ROOT/generated-rust-names.txt" <<'EXPECTED'
hl.canonical.v1.rs
hl.common.v1.rs
hl.health.v1.rs
hl.stream.v1.rs
EXPECTED
then
  printf 'generated-check:error generated Rust artifact contract changed\n' >&2
  exit 1
fi

CARGO_TARGET_DIR="$TARGET_COMPAT" \
  cargo +1.97.1 run -p schema-check --frozen --offline -- \
  check schemas/proto/baseline/v1.pb "$CONTRACT_A/current.pb"

SOURCE_DATE_EPOCH="$FIXED_EPOCH" CARGO_TARGET_DIR="$TARGET_BUILD_A" \
  cargo +1.97.1 run -p build-info --release --frozen --offline -- print \
  >"$TEMP_ROOT/build-a.json"
SOURCE_DATE_EPOCH="$FIXED_EPOCH" CARGO_TARGET_DIR="$TARGET_BUILD_B" \
  cargo +1.97.1 run -p build-info --release --frozen --offline -- print \
  >"$TEMP_ROOT/build-b.json"
cmp "$TEMP_ROOT/build-a.json" "$TEMP_ROOT/build-b.json"

EXPECTED_GIT_SHA="$(git rev-parse HEAD)"
readonly EXPECTED_GIT_SHA
EXPECTED_LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
readonly EXPECTED_LOCK_SHA
EXPECTED_SCHEMA_SHA="$(
  tail -n +2 crates/telemetry/schema-fingerprint-v1.material |
    tr -d '\n' |
    xxd -r -p |
    shasum -a 256 |
    awk '{print $1}'
)"
readonly EXPECTED_SCHEMA_SHA
EXPECTED_TARGET="$(rustc +1.97.1 -vV | awk '/^host:/ {print $2}')"
readonly EXPECTED_TARGET

jq -e \
  --arg git_sha "$EXPECTED_GIT_SHA" \
  --arg lock_sha "$EXPECTED_LOCK_SHA" \
  --arg schema_sha "$EXPECTED_SCHEMA_SHA" \
  --arg target "$EXPECTED_TARGET" \
  --arg rustc_version "$(rustc +1.97.1 --version)" \
  --argjson epoch "$FIXED_EPOCH" \
  '
    .git_sha == $git_sha and
    .dirty == false and
    .rustc_version == $rustc_version and
    .target_triple == $target and
    .build_epoch == $epoch and
    .reproducible == true and
    .schema_fingerprint == $schema_sha and
    .cargo_lock_sha256 == $lock_sha
  ' "$TEMP_ROOT/build-a.json" >/dev/null

git diff --exit-code -- fixtures/golden/manifest.toml

printf 'generated-check:ok git=%s epoch=%s descriptor_sha256=%s schema_sha256=%s\n' \
  "$EXPECTED_GIT_SHA" \
  "$FIXED_EPOCH" \
  "$(shasum -a 256 "$CONTRACT_A/current.pb" | awk '{print $1}')" \
  "$EXPECTED_SCHEMA_SHA"
