#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
gate="$repo_root/tools/ci/check-unsafe.sh"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/alpha-desk-unsafe.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT

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
