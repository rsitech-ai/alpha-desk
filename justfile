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
    ./tools/ci/check-dependency-exceptions.sh
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

oss-audit:
    cargo +1.97.1 run -p open-source-audit --locked --offline -- check --policy config/open-source-policy.toml --root .

spool-verify path="fixtures/spool/valid-v1":
    cargo +1.97.1 run -p spool-inspect --locked --offline -- verify {{quote(path)}}

archive-verify path="fixtures/archive/valid-v1":
    cargo +1.97.1 run -p archive-inspect --locked --offline -- verify {{quote(path)}}

archive-count path="fixtures/archive/valid-v1":
    cargo +1.97.1 run -p archive-inspect --locked --offline -- count {{quote(path)}}

spool-fuzz seconds="60":
    cargo +nightly-2026-07-16 fuzz run spool_segment fixtures/spool/valid-v1 -- -max_total_time={{quote(seconds)}}

stage-0-validate-config:
    cargo +1.97.1 run -p stage-gate --locked --offline -- validate-config config/stage-gates/stage-0.toml --schema config/stage-gates/schema-v1.json

stage-0-gate builder_id:
    cargo +1.97.1 run -p stage-gate --locked --offline -- run config/stage-gates/stage-0.toml --output target/stage-gates/stage-0.json --builder-id {{quote(builder_id)}}

stage-0-compose-smoke:
    ./tools/ci/stage-0-compose-smoke.sh

postgres-migration-smoke:
    ./tools/ci/check-postgres-migrations.sh

capture-e2e:
    ./tools/ci/capture-e2e.sh

capture-outage-e2e:
    CAPTURE_E2E_BLOCKS=5 CAPTURE_E2E_OUTAGE_MODE=nats-postgres ./tools/ci/capture-e2e.sh

capture-failover-e2e:
    CAPTURE_E2E_BLOCKS=5 CAPTURE_E2E_FAILOVER_MODE=1 ./tools/ci/capture-e2e.sh

capture-soak duration="10m":
    DURATION={{quote(duration)}} ./tools/ci/capture-soak.sh

state-replay-e2e blocks="100" checkpoint_after="50" iterations="3":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay/$run_id"; mkdir -p "target/evidence/state-replay"; cargo +1.97.1 run -p state-replay --locked --offline -- fixture-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-report:%s/report.json\n' "$output"

state-replay-soak blocks="1000" checkpoint_after="500" iterations="100":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay/$run_id"; mkdir -p "target/evidence/state-replay"; cargo +1.97.1 run --release -p state-replay --locked --offline -- fixture-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-soak-report:%s/report.json\n' "$output"

state-replay-trade-e2e blocks="100" checkpoint_after="50" iterations="3":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-trade/$run_id"; mkdir -p "target/evidence/state-replay-trade"; cargo +1.97.1 run -p state-replay --locked --offline -- trade-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-trade-report:%s/report.json\n' "$output"

state-replay-trade-soak blocks="1000" checkpoint_after="500" iterations="100":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-trade/$run_id"; mkdir -p "target/evidence/state-replay-trade"; cargo +1.97.1 run --release -p state-replay --locked --offline -- trade-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-trade-soak-report:%s/report.json\n' "$output"

state-replay-order-e2e blocks="100" checkpoint_after="50" iterations="3":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-order/$run_id"; mkdir -p "target/evidence/state-replay-order"; cargo +1.97.1 run -p state-replay --locked --offline -- order-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-order-report:%s/report.json\n' "$output"

state-replay-order-soak blocks="1000" checkpoint_after="500" iterations="100":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-order/$run_id"; mkdir -p "target/evidence/state-replay-order"; cargo +1.97.1 run --release -p state-replay --locked --offline -- order-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-order-soak-report:%s/report.json\n' "$output"

state-replay-market-e2e blocks="100" checkpoint_after="50" iterations="3":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-market/$run_id"; mkdir -p "target/evidence/state-replay-market"; cargo +1.97.1 run -p state-replay --locked --offline -- market-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-market-report:%s/report.json\n' "$output"

state-replay-market-soak blocks="1000" checkpoint_after="500" iterations="100":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-market/$run_id"; mkdir -p "target/evidence/state-replay-market"; cargo +1.97.1 run --release -p state-replay --locked --offline -- market-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-market-soak-report:%s/report.json\n' "$output"

state-replay-account-e2e blocks="100" checkpoint_after="50" iterations="3":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-account/$run_id"; mkdir -p "target/evidence/state-replay-account"; cargo +1.97.1 run -p state-replay --locked --offline -- account-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-account-report:%s/report.json\n' "$output"

state-replay-account-soak blocks="1000" checkpoint_after="500" iterations="100":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-account/$run_id"; mkdir -p "target/evidence/state-replay-account"; cargo +1.97.1 run --release -p state-replay --locked --offline -- account-e2e --output "$output" --blocks {{quote(blocks)}} --checkpoint-after {{quote(checkpoint_after)}} --iterations {{quote(iterations)}}; printf 'state-replay-account-soak-report:%s/report.json\n' "$output"

state-replay-archive-e2e archive chain start_height end_height checkpoint_height iterations="3":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-archive/$run_id"; mkdir -p "target/evidence/state-replay-archive"; cargo +1.97.1 run -p state-replay --locked --offline -- archive-e2e --archive {{quote(archive)}} --output "$output" --chain {{quote(chain)}} --start-height {{quote(start_height)}} --end-height {{quote(end_height)}} --checkpoint-height {{quote(checkpoint_height)}} --iterations {{quote(iterations)}}; printf 'state-replay-archive-report:%s/report.json\n' "$output"

state-replay-archive-soak archive chain start_height end_height checkpoint_height iterations="100":
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"; output="target/evidence/state-replay-archive/$run_id"; mkdir -p "target/evidence/state-replay-archive"; cargo +1.97.1 run --release -p state-replay --locked --offline -- archive-e2e --archive {{quote(archive)}} --output "$output" --chain {{quote(chain)}} --start-height {{quote(start_height)}} --end-height {{quote(end_height)}} --checkpoint-height {{quote(checkpoint_height)}} --iterations {{quote(iterations)}}; printf 'state-replay-archive-soak-report:%s/report.json\n' "$output"

dev-up:
    ./tools/dev/with-dev-secrets.sh docker compose -f infra/docker-compose/compose.yaml up -d
    ./tools/dev/with-dev-secrets.sh ./tools/ci/wait-for-dev-stack.sh
    ./tools/dev/with-dev-secrets.sh docker compose -f infra/docker-compose/compose.yaml run --rm --no-deps --entrypoint /bin/sh nats-init /opt/alpha-desk/test-permissions.sh

dev-down:
    ./tools/dev/with-dev-secrets.sh docker compose -f infra/docker-compose/compose.yaml down --timeout 60 --remove-orphans

dev-reset:
    printf '%s\n' 'WARNING: dev-reset destroys all alpha-desk-dev local data volumes.' >&2
    ./tools/dev/with-dev-secrets.sh docker compose -f infra/docker-compose/compose.yaml down --timeout 60 --volumes --remove-orphans
