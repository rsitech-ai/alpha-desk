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
    '^(paste v1\.0\.15 \(proc-macro\)|parquet v58\.4\.0|datafusion(-[a-z-]+)? v54\.1\.0( \(\*\))?|hl-analytics v0\.1\.0 \(.+\)|archive-inspect v0\.1\.0 \(.+\))$' \
    <<<"$paste_tree" | grep -q .; then
    printf 'dependency-exception-error: unexpected paste inverse dependency:\n%s\n' "$paste_tree" >&2
    exit 1
fi

if ! grep -Fxq '  "RUSTSEC-2024-0436",' deny.toml; then
    printf '%s\n' 'dependency-exception-error: paste advisory exception is missing' >&2
    exit 1
fi

printf '%s\n' 'dependency-exceptions:ok paste=1.0.15 parquet=58.4.0'
