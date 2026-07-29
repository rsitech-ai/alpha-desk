#!/usr/bin/env bash
set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
readonly repository_root
readonly environment_file="${repository_root}/state/dev/nats.env"

"${repository_root}/tools/dev/ensure-nats-dev-credentials.sh"
[[ -f "$environment_file" && ! -L "$environment_file" ]] || {
  printf '%s\n' 'dev-secrets:error NATS environment file is unavailable' >&2
  exit 1
}

set -a
# shellcheck disable=SC1090
source "$environment_file"
set +a

exec "$@"
