#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 1 ]]; then
  printf 'invalid service instance: expected exactly one argument\n' >&2
  exit 64
fi

case "$1" in
  hl-capture | hl-core | hl-analytics | hl-research | hl-api)
    printf '%s\n' "$1"
    ;;
  *)
    printf 'invalid service instance: %s\n' "$1" >&2
    exit 65
    ;;
esac
