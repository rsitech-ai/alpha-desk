#!/usr/bin/env bash
set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
readonly repository_root
readonly secret_directory="${repository_root}/state/dev"
readonly environment_file="${secret_directory}/nats.env"
readonly capture_password_file="${secret_directory}/nats-capture.password"

file_mode() {
  if stat -f '%Lp' "$1" >/dev/null 2>&1; then
    stat -f '%Lp' "$1"
  else
    stat -c '%a' "$1"
  fi
}

for command_name in git openssl; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'nats-dev-credentials:error missing command %s\n' "$command_name" >&2
    exit 2
  }
done

if [[ -L "$secret_directory" || -L "$environment_file" || -L "$capture_password_file" ]]; then
  printf '%s\n' 'nats-dev-credentials:error secret paths must not be symlinks' >&2
  exit 1
fi

umask 077
mkdir -p -- "$secret_directory"
chmod 700 "$secret_directory"

if [[ -f "$environment_file" && -f "$capture_password_file" ]]; then
  [[ "$(file_mode "$environment_file")" == 600 ]] || {
    printf '%s\n' 'nats-dev-credentials:error nats.env mode must be 600' >&2
    exit 1
  }
  [[ "$(file_mode "$capture_password_file")" == 600 ]] || {
    printf '%s\n' 'nats-dev-credentials:error capture password mode must be 600' >&2
    exit 1
  }
  exit 0
fi

if [[ -e "$environment_file" || -e "$capture_password_file" ]]; then
  printf '%s\n' 'nats-dev-credentials:error partial credential set; preserve evidence and repair explicitly' >&2
  exit 1
fi

bootstrap_password="$(openssl rand -hex 32)"
capture_password="$(openssl rand -hex 32)"
reader_password="$(openssl rand -hex 32)"

printf '%s' "$capture_password" >"$capture_password_file"
chmod 600 "$capture_password_file"
{
  printf 'export ALPHA_DESK_NATS_BOOTSTRAP_USER=%q\n' 'alpha_bootstrap'
  printf 'export ALPHA_DESK_NATS_BOOTSTRAP_PASSWORD=%q\n' "$bootstrap_password"
  printf 'export ALPHA_DESK_NATS_CAPTURE_USER=%q\n' 'alpha_capture'
  printf 'export ALPHA_DESK_NATS_CAPTURE_PASSWORD=%q\n' "$capture_password"
  printf 'export ALPHA_DESK_NATS_READER_USER=%q\n' 'alpha_reader'
  printf 'export ALPHA_DESK_NATS_READER_PASSWORD=%q\n' "$reader_password"
} >"$environment_file"
chmod 600 "$environment_file"

printf 'nats-dev-credentials:ok directory=%s\n' "$secret_directory"
