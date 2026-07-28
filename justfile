set shell := ["bash", "-euo", "pipefail", "-c"]

fmt:
    cargo +1.97.1 fmt --all -- --check

clippy:
    cargo +1.97.1 clippy --workspace --all-targets --all-features --locked --offline -- -D warnings

ci-test:
    version="$(cargo +1.97.1 nextest --version)"; if [[ "$version" != cargo-nextest\ 0.9.140\ * ]]; then printf 'cargo-nextest-version-error: expected cargo-nextest 0.9.140, got %q\n' "$version" >&2; exit 1; fi
    cargo +1.97.1 nextest run --workspace --all-features --locked --offline
    cargo +1.97.1 test --workspace --all-features --doc --locked --offline

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

generated:
    ./tools/ci/check-generated.sh

reproducible:
    ./tools/ci/verify-reproducible-build.sh

reproducible-environment:
    CC=/tmp/ambient-cc-must-not-run CFLAGS=-Dambient_cflags_must_not_apply CARGO_PROFILE_RELEASE_LTO=false RUSTC_WRAPPER=/tmp/ambient-rustc-wrapper-must-not-run ./tools/ci/verify-reproducible-build.sh --check-environment-seal

ci-verify: check-workspace quality ci-test

verify: check-workspace quality test

stage-0-validate-config:
    cargo +1.97.1 run -p stage-gate --locked --offline -- validate-config config/stage-gates/stage-0.toml --schema config/stage-gates/schema-v1.json

stage-0-gate builder_id:
    cargo +1.97.1 run -p stage-gate --locked --offline -- run config/stage-gates/stage-0.toml --output target/stage-gates/stage-0.json --builder-id {{quote(builder_id)}}

stage-0-compose-smoke:
    ./tools/ci/stage-0-compose-smoke.sh

dev-up:
    docker compose -f infra/docker-compose/compose.yaml up -d --wait --wait-timeout 120
    ./tools/ci/wait-for-dev-stack.sh

dev-down:
    docker compose -f infra/docker-compose/compose.yaml down --timeout 60 --remove-orphans

dev-reset:
    printf '%s\n' 'WARNING: dev-reset destroys all alpha-desk-dev local data volumes.' >&2
    docker compose -f infra/docker-compose/compose.yaml down --timeout 60 --volumes --remove-orphans
