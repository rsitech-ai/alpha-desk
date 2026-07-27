set shell := ["bash", "-euo", "pipefail", "-c"]

fmt:
    cargo +1.97.1 fmt --all -- --check

clippy:
    cargo +1.97.1 clippy --workspace --all-targets --all-features --locked --offline -- -D warnings

test:
    cargo +1.97.1 test --workspace --all-features --locked --offline
    swift test --package-path apps/AlphaDesk

check-workspace:
    ./tools/ci/check-workspace.sh

architecture:
    cargo +1.97.1 run -p architecture-check --locked --offline -- check
    ./tools/ci/check-unsafe.sh

deny:
    version="$(cargo +1.97.1 deny --version)"; if [[ "$version" != "cargo-deny 0.20.2" ]]; then printf 'cargo-deny-version-error: expected cargo-deny 0.20.2, got %q\n' "$version" >&2; exit 1; fi
    cargo +1.97.1 deny --locked --offline check

quality: fmt clippy architecture deny

verify: check-workspace quality test
