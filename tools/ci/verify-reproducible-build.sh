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

mode=build
if [[ "${1:-}" == "--check-environment-seal" && "$#" -eq 1 ]]; then
  mode=check-environment-seal
elif [[ "$#" -ne 0 ]]; then
  printf 'usage: %s [--check-environment-seal]\n' "$0" >&2
  exit 2
fi
readonly mode

case "${SOURCE_DATE_EPOCH:-}" in
  ''|*[!0-9]*)
    printf 'reproducible-build:error SOURCE_DATE_EPOCH must be an unsigned integer\n' >&2
    exit 2
    ;;
esac

for tool in awk cargo cc cmp diff dirname env git grep jq mktemp rm rustc rustup sed shasum sort stat; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'reproducible-build:error required tool is unavailable: %s\n' "$tool" >&2
    exit 2
  fi
done

if ! grep -Fx 'rustflags = ["-Dwarnings"]' "$REPO_ROOT/.cargo/config.toml" >/dev/null; then
  printf 'reproducible-build:error expected repository rustflags contract is missing\n' >&2
  exit 2
fi

TOOLCHAIN_CARGO_PATH="$(rustup which --toolchain 1.97.1 cargo)"
readonly TOOLCHAIN_CARGO_PATH
TOOLCHAIN_RUSTC_PATH="$(rustup which --toolchain 1.97.1 rustc)"
readonly TOOLCHAIN_RUSTC_PATH
readonly TOOLCHAIN_BIN_DIR="${TOOLCHAIN_CARGO_PATH%/*}"
HOST_TRIPLE="$("$TOOLCHAIN_RUSTC_PATH" -vV | awk '/^host:/ {print $2}')"
readonly HOST_TRIPLE
LINKER_PATH="$(command -v cc)"
readonly LINKER_PATH
TEMP_ROOT_CANDIDATE="$(mktemp -d "${TMPDIR:-/tmp}/alpha-desk-repro.XXXXXX")"
readonly TEMP_ROOT_CANDIDATE
TEMP_ROOT="$(
  CDPATH='' builtin cd -- "$TEMP_ROOT_CANDIDATE" &&
    builtin pwd -P
)"
readonly TEMP_ROOT
readonly TREE_A="$TEMP_ROOT/source-a"
readonly TREE_B="$TEMP_ROOT/source-b"
readonly TARGET_A="$TEMP_ROOT/target-a"
readonly TARGET_B="$TEMP_ROOT/target-b"
readonly MANIFEST_A="$TEMP_ROOT/manifest-a.txt"
readonly MANIFEST_B="$TEMP_ROOT/manifest-b.txt"
readonly SEALED_HOME="$TEMP_ROOT/home"
readonly SEALED_CARGO_HOME="$TEMP_ROOT/cargo-home"
readonly SEALED_TMPDIR="$TEMP_ROOT/tmp"
SEALED_RUSTUP_HOME="${RUSTUP_HOME:-${HOME:?}/.rustup}"
readonly SEALED_RUSTUP_HOME
readonly SEALED_PATH="$TOOLCHAIN_BIN_DIR:/usr/bin:/bin:/usr/sbin:/sbin"
worktree_a_added=false
worktree_b_added=false
mkdir -p "$SEALED_HOME" "$SEALED_CARGO_HOME" "$SEALED_TMPDIR"

cleanup() {
  local status=$?
  if [[ "$worktree_a_added" == true ]]; then
    git -C "$REPO_ROOT" worktree remove --force "$TREE_A" >/dev/null 2>&1 || true
  fi
  if [[ "$worktree_b_added" == true ]]; then
    git -C "$REPO_ROOT" worktree remove --force "$TREE_B" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$TEMP_ROOT"
  exit "$status"
}
trap cleanup EXIT INT TERM

run_sealed_environment() {
  local tree=$1
  local target=$2
  local offline=$3
  shift 3
  local rustflags
  rustflags="-Dwarnings -C linker=$LINKER_PATH"
  rustflags+=" --remap-path-prefix=$tree=/alpha-desk/source"
  rustflags+=" --remap-path-prefix=$target=/alpha-desk/target"
  rustflags+=" --remap-path-prefix=$SEALED_CARGO_HOME=/alpha-desk/cargo-home"
  rustflags+=" --remap-path-prefix=$SEALED_HOME=/alpha-desk/home"
  rustflags+=" --remap-path-prefix=$SEALED_RUSTUP_HOME=/alpha-desk/rustup-home"
  rustflags+=" --remap-path-prefix=$SEALED_TMPDIR=/alpha-desk/tmp"

  env -i \
    CARGO_HOME="$SEALED_CARGO_HOME" \
    CARGO_INCREMENTAL=0 \
    CARGO_NET_OFFLINE="$offline" \
    CARGO_TARGET_DIR="$target" \
    CARGO_TERM_COLOR=never \
    GIT_CONFIG_GLOBAL=/dev/null \
    GIT_CONFIG_NOSYSTEM=1 \
    HOME="$SEALED_HOME" \
    LANG=C \
    LC_ALL=C \
    PATH="$SEALED_PATH" \
    RUSTC="$TOOLCHAIN_RUSTC_PATH" \
    RUSTFLAGS="$rustflags" \
    RUSTUP_HOME="$SEALED_RUSTUP_HOME" \
    SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
    TMPDIR="$SEALED_TMPDIR" \
    TZ=UTC \
    ZERO_AR_DATE=1 \
    "$@"
}

check_environment_seal() {
  local actual="$TEMP_ROOT/environment.txt"
  local actual_names="$TEMP_ROOT/environment-names.txt"
  local expected_names="$TEMP_ROOT/expected-environment-names.txt"

  run_sealed_environment \
    "$REPO_ROOT" \
    "$TEMP_ROOT/environment-target" \
    true \
    /usr/bin/env >"$actual"
  sed 's/=.*//' "$actual" | LC_ALL=C sort >"$actual_names"
  LC_ALL=C sort >"$expected_names" <<'EXPECTED'
CARGO_HOME
CARGO_INCREMENTAL
CARGO_NET_OFFLINE
CARGO_TARGET_DIR
CARGO_TERM_COLOR
GIT_CONFIG_GLOBAL
GIT_CONFIG_NOSYSTEM
HOME
LANG
LC_ALL
PATH
RUSTC
RUSTFLAGS
RUSTUP_HOME
SOURCE_DATE_EPOCH
TMPDIR
TZ
ZERO_AR_DATE
EXPECTED
  if ! diff -u "$expected_names" "$actual_names"; then
    printf 'reproducible-build:error sealed environment allowlist changed\n' >&2
    exit 1
  fi
  grep -Fx 'CARGO_NET_OFFLINE=true' "$actual" >/dev/null
  grep -Fx 'CARGO_INCREMENTAL=0' "$actual" >/dev/null
  if grep -Eq \
    '^(AR|CC|CFLAGS|CXX|CXXFLAGS|LDFLAGS|RANLIB|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|CARGO_ENCODED_RUSTFLAGS|CARGO_PROFILE_RELEASE_[^=]*)=' \
    "$actual"; then
    printf 'reproducible-build:error hostile ambient build variable crossed the seal\n' >&2
    exit 1
  fi
  printf '%s\n' \
    'reproducible-build:environment-seal allowlist=CARGO_HOME,CARGO_INCREMENTAL,CARGO_NET_OFFLINE,CARGO_TARGET_DIR,CARGO_TERM_COLOR,GIT_CONFIG_GLOBAL,GIT_CONFIG_NOSYSTEM,HOME,LANG,LC_ALL,PATH,RUSTC,RUSTFLAGS,RUSTUP_HOME,SOURCE_DATE_EPOCH,TMPDIR,TZ,ZERO_AR_DATE'
  printf '%s\n' \
    'reproducible-build:environment-seal stripped=AR,CC,CFLAGS,CXX,CXXFLAGS,LDFLAGS,RANLIB,RUSTC_WRAPPER,RUSTC_WORKSPACE_WRAPPER,CARGO_ENCODED_RUSTFLAGS,CARGO_PROFILE_RELEASE_*'
  printf 'reproducible-build:environment-seal ok\n'
}

if [[ "$mode" == check-environment-seal ]]; then
  check_environment_seal
  exit 0
fi

if [[ -n "$(git -C "$REPO_ROOT" status --porcelain=v1 --untracked-files=all)" ]]; then
  printf 'reproducible-build:error repository must be clean\n' >&2
  exit 2
fi

COMMIT_SHA="$(git -C "$REPO_ROOT" rev-parse HEAD)"
readonly COMMIT_SHA

run_sealed_environment \
  "$REPO_ROOT" \
  "$TEMP_ROOT/fetch-target" \
  false \
  "$TOOLCHAIN_CARGO_PATH" fetch --locked \
    --manifest-path "$REPO_ROOT/Cargo.toml"
git -C "$REPO_ROOT" worktree add --detach "$TREE_A" "$COMMIT_SHA" >/dev/null
worktree_a_added=true
git -C "$REPO_ROOT" worktree add --detach "$TREE_B" "$COMMIT_SHA" >/dev/null
worktree_b_added=true

build_tree() {
  local tree=$1
  local target=$2

  (
    cd -- "$tree"
    run_sealed_environment \
      "$tree" \
      "$target" \
      true \
      "$TOOLCHAIN_CARGO_PATH" build --workspace --bins --release --all-features --frozen \
        --target "$HOST_TRIPLE"
  )
}

service_binaries() {
  local tree=$1
  (
    cd -- "$tree"
    run_sealed_environment \
      "$tree" \
      "$TEMP_ROOT/metadata-target" \
      true \
      "$TOOLCHAIN_CARGO_PATH" metadata --no-deps --format-version 1 --frozen |
      jq -r \
        --arg prefix "$tree/services/" \
        '
          .packages[]
          | select(.manifest_path | startswith($prefix))
          | .targets[]
          | select(.kind | index("bin"))
          | .name
        ' |
      LC_ALL=C sort -u
  )
}

assert_service_binary_contract() {
  local tree=$1
  local actual="$TEMP_ROOT/service-binaries.txt"
  service_binaries "$tree" >"$actual"
  if ! diff -u - "$actual" <<'EXPECTED'
hl-analytics
hl-api
hl-capture
hl-core
hl-research
EXPECTED
  then
    printf 'reproducible-build:error service binary contract changed\n' >&2
    exit 1
  fi
}

artifact_size() {
  local artifact=$1
  if stat -f '%z' "$artifact" >/dev/null 2>&1; then
    stat -f '%z' "$artifact"
  else
    stat -c '%s' "$artifact"
  fi
}

write_manifest() {
  local tree=$1
  local target=$2
  local output=$3
  local name artifact

  : >"$output"
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    artifact="$target/$HOST_TRIPLE/release/$name"
    if [[ ! -f "$artifact" ]]; then
      printf 'reproducible-build:error declared service binary is missing: %s\n' "$name" >&2
      exit 1
    fi
    printf '%s\t%s\t%s\n' \
      "$name" \
      "$(artifact_size "$artifact")" \
      "$(shasum -a 256 "$artifact" | awk '{print $1}')" \
      >>"$output"
  done < <(service_binaries "$tree")

  LC_ALL=C sort -o "$output" "$output"
  if [[ ! -s "$output" ]]; then
    printf 'reproducible-build:error Cargo metadata declared no service binaries\n' >&2
    exit 1
  fi
}

assert_service_binary_contract "$TREE_A"
assert_service_binary_contract "$TREE_B"
build_tree "$TREE_A" "$TARGET_A"
build_tree "$TREE_B" "$TARGET_B"
write_manifest "$TREE_A" "$TARGET_A" "$MANIFEST_A"
write_manifest "$TREE_B" "$TARGET_B" "$MANIFEST_B"

if ! cmp -s "$MANIFEST_A" "$MANIFEST_B"; then
  printf 'reproducible-build:error artifact manifests differ\n' >&2
  diff -u "$MANIFEST_A" "$MANIFEST_B" >&2 || true
  exit 1
fi

while IFS=$'\t' read -r name _size _digest; do
  artifact_a="$TARGET_A/$HOST_TRIPLE/release/$name"
  artifact_b="$TARGET_B/$HOST_TRIPLE/release/$name"
  if ! cmp -s "$artifact_a" "$artifact_b"; then
    printf 'reproducible-build:error binary differs: %s\n' "$name" >&2
    printf 'first size=%s sha256=%s\n' \
      "$(artifact_size "$artifact_a")" \
      "$(shasum -a 256 "$artifact_a" | awk '{print $1}')" >&2
    printf 'second size=%s sha256=%s\n' \
      "$(artifact_size "$artifact_b")" \
      "$(shasum -a 256 "$artifact_b" | awk '{print $1}')" >&2
    cmp -l "$artifact_a" "$artifact_b" >"$TEMP_ROOT/$name.cmp" || true
    awk 'NR <= 64 {print}' "$TEMP_ROOT/$name.cmp" >&2
    exit 1
  fi
done <"$MANIFEST_A"

printf 'reproducible-build:evidence commit=%s epoch=%s host=%s\n' \
  "$COMMIT_SHA" "$SOURCE_DATE_EPOCH" "$HOST_TRIPLE"
printf 'reproducible-build:linker path=%s\n' "$LINKER_PATH"
"$TOOLCHAIN_RUSTC_PATH" -vV
"$TOOLCHAIN_CARGO_PATH" -V
"$LINKER_PATH" --version 2>&1 | awk 'NR <= 2 {print}'
printf 'reproducible-build:toolchain cargo=%s rustc=%s\n' \
  "$TOOLCHAIN_CARGO_PATH" "$TOOLCHAIN_RUSTC_PATH"
printf 'reproducible-build:environment home=%s cargo_home=%s rustup_home=%s tmpdir=%s path=%s\n' \
  "$SEALED_HOME" "$SEALED_CARGO_HOME" "$SEALED_RUSTUP_HOME" "$SEALED_TMPDIR" "$SEALED_PATH"
printf '%s\n' \
  'reproducible-build:environment-allowlist CARGO_HOME,CARGO_INCREMENTAL,CARGO_NET_OFFLINE,CARGO_TARGET_DIR,CARGO_TERM_COLOR,GIT_CONFIG_GLOBAL,GIT_CONFIG_NOSYSTEM,HOME,LANG,LC_ALL,PATH,RUSTC,RUSTFLAGS,RUSTUP_HOME,SOURCE_DATE_EPOCH,TMPDIR,TZ,ZERO_AR_DATE'
printf '%s\n' \
  'reproducible-build:environment-values CARGO_INCREMENTAL=0 CARGO_NET_OFFLINE=true CARGO_TERM_COLOR=never GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 LANG=C LC_ALL=C TZ=UTC ZERO_AR_DATE=1'
printf 'reproducible-build:rustflags-a %s\n' \
  "-Dwarnings -C linker=$LINKER_PATH --remap-path-prefix=$TREE_A=/alpha-desk/source --remap-path-prefix=$TARGET_A=/alpha-desk/target --remap-path-prefix=$SEALED_CARGO_HOME=/alpha-desk/cargo-home --remap-path-prefix=$SEALED_HOME=/alpha-desk/home --remap-path-prefix=$SEALED_RUSTUP_HOME=/alpha-desk/rustup-home --remap-path-prefix=$SEALED_TMPDIR=/alpha-desk/tmp"
printf 'reproducible-build:rustflags-b %s\n' \
  "-Dwarnings -C linker=$LINKER_PATH --remap-path-prefix=$TREE_B=/alpha-desk/source --remap-path-prefix=$TARGET_B=/alpha-desk/target --remap-path-prefix=$SEALED_CARGO_HOME=/alpha-desk/cargo-home --remap-path-prefix=$SEALED_HOME=/alpha-desk/home --remap-path-prefix=$SEALED_RUSTUP_HOME=/alpha-desk/rustup-home --remap-path-prefix=$SEALED_TMPDIR=/alpha-desk/tmp"
printf 'reproducible-build:profile cargo_profile=release workspace=true bins=true all_features=true frozen=true target=%s\n' \
  "$HOST_TRIPLE"
printf 'reproducible-build:artifacts\n'
sed 's/^/  /' "$MANIFEST_A"
printf 'reproducible-build:ok\n'
