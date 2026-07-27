#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
gate="$repo_root/tools/ci/check-unsafe.sh"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/alpha-desk-unsafe.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT
caller_cargo_home="${CARGO_HOME:-${HOME:?HOME must identify the caller cache}/.cargo}"
caller_rustup_home="${RUSTUP_HOME:-${HOME:?HOME must identify rustup}/.rustup}"

mkdir -p "$fixture_root/src"
cat >"$fixture_root/Cargo.toml" <<'EOF'
[package]
name = "unsafe-gate-fixture"
version = "0.1.0"
edition = "2024"
EOF

cat >"$fixture_root/src/lib.rs" <<'EOF'
// The text `unsafe {}` in a comment is not unsafe Rust.
pub const DESCRIPTION: &str = "unsafe { core::hint::unreachable_unchecked() }";
EOF

cargo +1.97.1 generate-lockfile --manifest-path "$fixture_root/Cargo.toml" --offline
if ! "$gate" --manifest-path "$fixture_root/Cargo.toml"; then
  echo "safe comments and strings must pass the compiler gate" >&2
  exit 1
fi

relocated_cargo_home="$fixture_root/relocated-cargo-home"
mkdir -p "$relocated_cargo_home/registry"
for cache_input in registry/index registry/cache; do
  ln -s "$caller_cargo_home/$cache_input" "$relocated_cargo_home/$cache_input"
done
set +e
HOME="" \
  RUSTUP_HOME="$caller_rustup_home" \
  CARGO_HOME="$relocated_cargo_home" \
  "$gate" --manifest-path "$fixture_root/Cargo.toml"
relocated_cache_status=$?
set -e
if ((relocated_cache_status != 0)); then
  echo "a legitimate relocated Cargo cache must satisfy the offline gate" >&2
  exit 1
fi

cat >"$fixture_root/src/lib.rs" <<'EOF'
pub fn read(value: &u8) -> u8 {
    unsafe { core::ptr::read_volatile(value) }
}
EOF

if "$gate" --manifest-path "$fixture_root/Cargo.toml"; then
  echo "a real unsafe block must fail the compiler gate" >&2
  exit 1
fi

cat >"$fixture_root/src/lib.rs" <<'EOF'
#![allow(unsafe_code)]

pub fn read(value: &u8) -> u8 {
    unsafe { core::ptr::read_volatile(value) }
}
EOF

if "$gate" --manifest-path "$fixture_root/Cargo.toml"; then
  echo "crate-level allow(unsafe_code) must not override the compiler gate" >&2
  exit 1
fi

hostile_cargo_home="$fixture_root/hostile-cargo-home"
hostile_wrapper="$fixture_root/strip-forbid-wrapper.sh"
hostile_wrapper_marker="$fixture_root/hostile-wrapper-ran"
poisoned_target="$fixture_root/poisoned-target"
mkdir -p "$hostile_cargo_home/registry"
for cache_input in registry/index registry/cache; do
  ln -s "$caller_cargo_home/$cache_input" "$hostile_cargo_home/$cache_input"
done

cat >"$hostile_wrapper" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

touch "$HOSTILE_WRAPPER_MARKER"
filtered=()
while (($# > 0)); do
  case "$1" in
    -Funsafe_code|-Funsafe-code)
      shift
      ;;
    -F)
      if (($# > 1)) && [[ "$2" == "unsafe_code" || "$2" == "unsafe-code" ]]; then
        shift 2
      else
        filtered+=("$1")
        shift
      fi
      ;;
    *)
      filtered+=("$1")
      shift
      ;;
  esac
done
exec "${filtered[@]}"
EOF
chmod +x "$hostile_wrapper"

cat >"$hostile_cargo_home/config.toml" <<EOF
[build]
rustc-wrapper = "$hostile_wrapper"
EOF

cargo_isolation_failures=0
if CARGO_HOME="$hostile_cargo_home" \
  CARGO_TARGET_DIR="$fixture_root/hostile-gate-target" \
  HOSTILE_WRAPPER_MARKER="$hostile_wrapper_marker" \
  "$gate" --manifest-path "$fixture_root/Cargo.toml"
then
  echo "Cargo-home rustc-wrapper must not strip the forbid lint" >&2
  cargo_isolation_failures=1
fi
if [[ -e "$hostile_wrapper_marker" ]]; then
  echo "Cargo-home rustc-wrapper must not execute in the unsafe gate" >&2
  cargo_isolation_failures=1
fi

rm -f "$hostile_wrapper_marker"
if ! CARGO_HOME="$hostile_cargo_home" \
  CARGO_TARGET_DIR="$poisoned_target" \
  HOSTILE_WRAPPER_MARKER="$hostile_wrapper_marker" \
  RUSTFLAGS=-Funsafe_code \
  cargo +1.97.1 check \
    --manifest-path "$fixture_root/Cargo.toml" \
    --all-targets \
    --all-features \
    --locked \
    --offline
then
  echo "the hostile wrapper must create the poisoned-target control artifact" >&2
  exit 1
fi
if [[ ! -e "$hostile_wrapper_marker" ]]; then
  echo "the poisoned-target control must prove the hostile wrapper executed" >&2
  exit 1
fi
rm -f "$hostile_wrapper_marker"

if CARGO_TARGET_DIR="$poisoned_target" \
  "$gate" --manifest-path "$fixture_root/Cargo.toml"
then
  echo "a caller-supplied poisoned target must not satisfy the unsafe gate" >&2
  cargo_isolation_failures=1
fi
if [[ -e "$hostile_wrapper_marker" ]]; then
  echo "the clean poisoned-target probe must not execute the hostile wrapper" >&2
  cargo_isolation_failures=1
fi
if ((cargo_isolation_failures != 0)); then
  exit 1
fi

if RUSTFLAGS=--cap-lints=allow \
  "$gate" --manifest-path "$fixture_root/Cargo.toml"
then
  echo "ambient RUSTFLAGS must not cap the unsafe-code lint" >&2
  exit 1
fi

if CARGO_ENCODED_RUSTFLAGS=-Aunsafe_code \
  "$gate" --manifest-path "$fixture_root/Cargo.toml"
then
  echo "ambient CARGO_ENCODED_RUSTFLAGS must not allow unsafe code" >&2
  exit 1
fi

cat >"$fixture_root/rustc-wrapper.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
touch "$UNSAFE_GATE_WRAPPER_MARKER"
exec "$@"
EOF
chmod +x "$fixture_root/rustc-wrapper.sh"
wrapper_marker="$fixture_root/wrapper-ran"

if RUSTC_WRAPPER="$fixture_root/rustc-wrapper.sh" \
  UNSAFE_GATE_WRAPPER_MARKER="$wrapper_marker" \
  "$gate" --manifest-path "$fixture_root/Cargo.toml"
then
  echo "real unsafe code must fail when an ambient wrapper is configured" >&2
  exit 1
fi
if [[ -e "$wrapper_marker" ]]; then
  echo "ambient RUSTC_WRAPPER must not participate in the unsafe gate" >&2
  exit 1
fi

cat >"$fixture_root/non-empty-allowlist.toml" <<'EOF'
schema-version = 1

[[waivers]]
path = "src/lib.rs"
line-hash = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
reviewer = "reviewer"
rationale = "not permitted in Stage 0"
expiry = "2099-01-01"
EOF

if ALPHA_DESK_UNSAFE_ALLOWLIST="$fixture_root/non-empty-allowlist.toml" \
  "$gate" --manifest-path "$fixture_root/Cargo.toml"
then
  echo "a non-empty Stage 0 unsafe allowlist must fail closed" >&2
  exit 1
fi
