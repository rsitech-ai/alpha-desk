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
manifest_directory="$(
  CDPATH= builtin cd -- "$(command dirname -- "$manifest_path")" &&
    builtin pwd -P
)"
manifest_path="$manifest_directory/$(command basename -- "$manifest_path")"

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

if ! rustup_command="$(command -v rustup)" || [[ -z "$rustup_command" ]]; then
  echo "unsafe-gate-error: rustup is required to resolve Rust 1.97.1" >&2
  exit 1
fi
if ! trusted_rustc="$("$rustup_command" which --toolchain 1.97.1 rustc)" ||
  [[ ! -x "$trusted_rustc" ]]
then
  echo "unsafe-gate-error: cannot resolve the Rust 1.97.1 compiler" >&2
  exit 1
fi
if ! trusted_cargo="$("$rustup_command" which --toolchain 1.97.1 cargo)" ||
  [[ ! -x "$trusted_cargo" ]]
then
  echo "unsafe-gate-error: cannot resolve Cargo for Rust 1.97.1" >&2
  exit 1
fi
if [[ "$("$trusted_rustc" --version)" != "rustc 1.97.1 "* ]]; then
  echo "unsafe-gate-error: resolved compiler is not Rust 1.97.1" >&2
  exit 1
fi
if [[ "$("$trusted_cargo" --version)" != "cargo 1.97.1 "* ]]; then
  echo "unsafe-gate-error: resolved Cargo is not 1.97.1" >&2
  exit 1
fi

caller_cache_home_input="${CARGO_HOME:-${HOME:?HOME must identify the caller cache}/.cargo}"
if ! caller_cache_home="$(
  CDPATH= builtin cd -- "$caller_cache_home_input" 2>/dev/null &&
    builtin pwd -P
)" || [[ -z "$caller_cache_home" ]]; then
  printf \
    'unsafe-gate-error: caller Cargo home is missing or not a directory: %q\n' \
    "$caller_cache_home_input" >&2
  exit 1
fi

gate_root="$(mktemp -d "${TMPDIR:-/tmp}/alpha-desk-unsafe-gate.XXXXXX")"
cleanup() {
  rm -rf -- "$gate_root"
}
trap cleanup EXIT

isolated_cargo_home="$gate_root/cargo-home"
gate_target_dir="$gate_root/target"
gate_build_dir="$gate_root/build"
mkdir -p "$isolated_cargo_home/registry" "$gate_target_dir" "$gate_build_dir"

# Preserve only registry index and packaged-crate cache inputs. Cargo verifies
# packaged registry crates against the checksums in locked registry metadata
# while extracting fresh sources into the isolated Cargo home. Configuration,
# credentials, previously extracted sources, and build artifacts are excluded.
for cache_input in registry/index registry/cache; do
  if [[ ! -d "$caller_cache_home/$cache_input" ]]; then
    echo "unsafe-gate-error: offline Cargo cache input is missing: $cache_input" >&2
    exit 1
  fi
  ln -s "$caller_cache_home/$cache_input" "$isolated_cargo_home/$cache_input"
done

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

export CARGO="$trusted_cargo"
export CARGO_HOME="$isolated_cargo_home"
export CARGO_TARGET_DIR="$gate_target_dir"
export CARGO_BUILD_TARGET_DIR="$gate_target_dir"
export CARGO_BUILD_BUILD_DIR="$gate_build_dir"
export RUSTC="$trusted_rustc"
export CARGO_BUILD_RUSTC="$trusted_rustc"
export RUSTC_WRAPPER=""
export RUSTC_WORKSPACE_WRAPPER=""
export CARGO_BUILD_RUSTC_WRAPPER=""
export CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER=""
export RUSTFLAGS="-Funsafe_code"
cd "$repo_root"
"$trusted_cargo" check \
  --manifest-path "$manifest_path" \
  --workspace \
  --all-targets \
  --all-features \
  --locked \
  --offline
