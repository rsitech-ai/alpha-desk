#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
manifest_path="$repo_root/Cargo.toml"
allowlist_path="${ALPHA_DESK_UNSAFE_ALLOWLIST:-$repo_root/tools/ci/unsafe-allowlist.toml}"

if (($# != 0)); then
  if (($# != 2)) || [[ "$1" != "--manifest-path" ]]; then
    echo "usage: check-unsafe.sh [--manifest-path <Cargo.toml>]" >&2
    exit 2
  fi
  manifest_path="$2"
fi

if [[ ! -f "$manifest_path" ]]; then
  echo "unsafe-gate-error: Cargo manifest not found: $manifest_path" >&2
  exit 1
fi

python3 - "$allowlist_path" <<'PY'
import pathlib
import sys
import tomllib

path = pathlib.Path(sys.argv[1])
try:
    with path.open("rb") as handle:
        document = tomllib.load(handle)
except (OSError, tomllib.TOMLDecodeError) as error:
    raise SystemExit(f"unsafe-allowlist-error: {path}: {error}")

expected = {"schema-version": 1, "waivers": []}
if document != expected:
    raise SystemExit(
        "unsafe-allowlist-error: Stage 0 requires exactly "
        "`schema-version = 1` and an empty `waivers = []`"
    )
PY

compiler_environment=(
  CARGO_BUILD_RUSTC
  CARGO_BUILD_RUSTC_WRAPPER
  CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER
  CARGO_BUILD_RUSTFLAGS
  CARGO_ENCODED_RUSTFLAGS
  RUSTC
  RUSTC_WRAPPER
  RUSTC_WORKSPACE_WRAPPER
  RUSTFLAGS
)
for variable in "${compiler_environment[@]}"; do
  unset "$variable"
done

# Cargo also accepts target-specific rustflags via dynamically named
# CARGO_TARGET_<TRIPLE>_RUSTFLAGS variables. They are part of the same
# untrusted caller environment and must not participate in this gate.
while IFS= read -r variable; do
  case "$variable" in
    CARGO_TARGET_*_RUSTFLAGS)
      unset "$variable"
      ;;
  esac
done < <(compgen -e)

export RUSTFLAGS="-Dunsafe_code"
cargo +1.97.1 check \
  --manifest-path "$manifest_path" \
  --workspace \
  --all-targets \
  --all-features \
  --locked \
  --offline
