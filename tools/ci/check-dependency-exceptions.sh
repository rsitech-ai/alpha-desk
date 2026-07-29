#!/usr/bin/env bash
set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
cd "$repository_root"

paste_versions="$(
    awk '
        $0 == "name = \"paste\"" {
            if (getline <= 0) {
                exit 2
            }
            print
        }
    ' Cargo.lock
)"
if [[ "$paste_versions" != 'version = "1.0.15"' ]]; then
    printf 'dependency-exception-error: expected only paste 1.0.15, got %q\n' "$paste_versions" >&2
    exit 1
fi

paste_tree="$(
    CARGO_TERM_COLOR=never cargo +1.97.1 tree \
        -i paste@1.0.15 \
        --locked \
        --offline \
        --prefix none
)"
if ! grep -Fxq 'paste v1.0.15 (proc-macro)' <<<"$paste_tree"; then
    printf '%s\n' 'dependency-exception-error: paste 1.0.15 root is missing' >&2
    exit 1
fi
if ! grep -Fxq 'parquet v58.4.0' <<<"$paste_tree"; then
    printf '%s\n' 'dependency-exception-error: paste is not bound to parquet 58.4.0' >&2
    exit 1
fi
if grep -Ev \
    '^(paste v1\.0\.15 \(proc-macro\)|parquet v58\.4\.0|datafusion(-[a-z-]+)? v54\.1\.0( \(\*\))?|canonical-archive v0\.1\.0 \(.+\)|hl-analytics v0\.1\.0 \(.+\)|hl-capture v0\.1\.0 \(.+\)|archive-inspect v0\.1\.0 \(.+\)|spool-inspect v0\.1\.0 \(.+\))$' \
    <<<"$paste_tree" | grep -q .; then
    printf 'dependency-exception-error: unexpected paste inverse dependency:\n%s\n' "$paste_tree" >&2
    exit 1
fi

if ! grep -Fxq '  "RUSTSEC-2024-0436",' deny.toml; then
    printf '%s\n' 'dependency-exception-error: paste advisory exception is missing' >&2
    exit 1
fi

nats_transition_crates=(
    'block-buffer@0.10.4'
    'const-oid@0.9.6'
    'cpufeatures@0.2.17'
    'crypto-common@0.1.7'
    'digest@0.10.7'
    'rand@0.8.7'
    'rand@0.10.2'
    'rand_chacha@0.3.1'
    'rand_core@0.6.4'
    'rand_core@0.10.1'
    'sha2@0.10.9'
    'windows-sys@0.52.0'
)

for crate_spec in "${nats_transition_crates[@]}"; do
    crate_name="${crate_spec%@*}"
    crate_version="${crate_spec#*@}"
    if ! grep -Fq \
        "{ crate = \"$crate_spec\", reason = \"async-nats 0.50.0" \
        deny.toml; then
        printf 'dependency-exception-error: missing exact async-nats transition %s\n' \
            "$crate_spec" >&2
        exit 1
    fi
    inverse_tree="$(
        CARGO_TERM_COLOR=never cargo +1.97.1 tree \
            -i "$crate_spec" \
            --target all \
            --locked \
            --offline \
            --prefix none
    )"
    if ! grep -Eq '^async-nats v0\.50\.0( |$)' <<<"$inverse_tree" &&
        ! grep -Eq '^nkeys v0\.4\.5( |$)' <<<"$inverse_tree"; then
        printf \
            'dependency-exception-error: %s is no longer bound to async-nats/nkeys:\n%s\n' \
            "$crate_spec" "$inverse_tree" >&2
        exit 1
    fi
    if ! grep -Fxq "$crate_name v$crate_version" <<<"$inverse_tree"; then
        printf 'dependency-exception-error: missing inverse root %s\n' "$crate_spec" >&2
        exit 1
    fi
done

postgres_transition_crates=(
    'phf@0.13.1'
    'phf_shared@0.13.1'
    'wasi@0.14.7+wasi-0.2.4'
)

for crate_spec in "${postgres_transition_crates[@]}"; do
    crate_name="${crate_spec%@*}"
    crate_version="${crate_spec#*@}"
    if ! grep -Fq \
        "{ crate = \"$crate_spec\", reason = \"tokio-postgres 0.7.18" \
        deny.toml; then
        printf 'dependency-exception-error: missing exact tokio-postgres transition %s\n' \
            "$crate_spec" >&2
        exit 1
    fi
    inverse_tree="$(
        CARGO_TERM_COLOR=never cargo +1.97.1 tree \
            -i "$crate_spec" \
            --target all \
            --locked \
            --offline \
            --prefix none
    )"
    if ! grep -Eq '^tokio-postgres v0\.7\.18( |$)' <<<"$inverse_tree"; then
        printf \
            'dependency-exception-error: %s is no longer bound to tokio-postgres:\n%s\n' \
            "$crate_spec" "$inverse_tree" >&2
        exit 1
    fi
    if ! grep -Fxq "$crate_name v$crate_version" <<<"$inverse_tree"; then
        printf 'dependency-exception-error: missing inverse root %s\n' "$crate_spec" >&2
        exit 1
    fi
done

printf '%s\n' \
    'dependency-exceptions:ok paste=1.0.15 parquet=58.4.0 async-nats=0.50.0 tokio-postgres=0.7.18'
