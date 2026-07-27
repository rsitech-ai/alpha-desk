#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 0 ]]; then
  printf 'FAIL invocation:arguments-forbidden\n' >&2
  exit 64
fi

SCRIPT_DIR="$(
  CDPATH='' builtin cd -- "$(command dirname -- "${BASH_SOURCE[0]}")" &&
    builtin pwd -P
)"
readonly SCRIPT_DIR
readonly ANSIBLE_DIR="$SCRIPT_DIR/.."
REPO_ROOT="$(
  CDPATH='' builtin cd -- "$ANSIBLE_DIR/../.." &&
    builtin pwd -P
)"
readonly REPO_ROOT
readonly INVENTORY="$ANSIBLE_DIR/inventory/example/hosts.yml"
readonly PLAYBOOK="$SCRIPT_DIR/playbook.yml"
readonly LOCK="$ANSIBLE_DIR/requirements.lock"
readonly COLLECTION_REQUIREMENTS="$ANSIBLE_DIR/collections/requirements.yml"
readonly UBUNTU_IMAGE='ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90'
readonly PROMETHEUS_IMAGE='prom/prometheus:v3.13.1@sha256:3c42b892cf723fa54d2f262c37a0e1f80aa8c8ddb1da7b9b0df9455a35a7f893'
readonly LINUX_VERIFIER_IMAGE='alpha-desk-task9-linux-verifier:stage0'

failures=0
blocked_count=0
molecule_created=0
linux_image_created=0
tmp_root=''
venv=''

pass() {
  printf 'PASS %s\n' "$1"
}

fail() {
  printf 'FAIL %s\n' "$1" >&2
  failures=$((failures + 1))
}

blocked() {
  printf 'BLOCKED %s\n' "$1" >&2
  blocked_count=$((blocked_count + 1))
}

not_run() {
  printf 'NOT_RUN %s\n' "$1" >&2
}

run_gate() {
  local label=$1
  shift
  if "$@"; then
    pass "$label"
    return 0
  fi
  fail "$label"
  return 1
}

cleanup() {
  if [[ "$molecule_created" -eq 1 && -n "$venv" && -x "$venv/bin/molecule" ]]; then
    (
      cd "$ANSIBLE_DIR"
      MOLECULE_EPHEMERAL_DIRECTORY="$tmp_root/molecule" \
        env -u HOME "$venv/bin/molecule" destroy
    ) >/dev/null 2>&1 || true
  fi
  if [[ "$linux_image_created" -eq 1 ]]; then
    docker image rm "$LINUX_VERIFIER_IMAGE" >/dev/null 2>&1 || true
  fi
  if command -v docker >/dev/null 2>&1; then
    while IFS= read -r molecule_image_id; do
      if [[ "$molecule_image_id" =~ ^sha256:[0-9a-f]{64}$ ]]; then
        docker image rm "$molecule_image_id" >/dev/null 2>&1 || true
      fi
    done < <(
      docker images \
        --filter label=org.alpha-desk.task9-verifier=molecule \
        --format '{{.ID}}' 2>/dev/null |
        while IFS= read -r short_image_id; do
          docker image inspect "$short_image_id" --format '{{.Id}}' 2>/dev/null || true
        done
    )
  fi
  case "$tmp_root" in
    /private/tmp/alpha-task9-check.* | /tmp/alpha-task9-check.*)
      rm -rf -- "$tmp_root"
      ;;
  esac
}
trap cleanup EXIT

for ambient_name in \
  ALPHA_DESK_ENVIRONMENT \
  ALPHA_STAGE0_SCAFFOLD \
  ANSIBLE_ASYNC_DIR \
  ANSIBLE_COLLECTIONS_PATH \
  ANSIBLE_COLLECTIONS_PATHS \
  ANSIBLE_CONFIG \
  ANSIBLE_HOME \
  ANSIBLE_INVENTORY \
  ANSIBLE_LOCAL_TEMP \
  ANSIBLE_ROLES_PATH \
  MOLECULE_EPHEMERAL_DIRECTORY \
  MOLECULE_FILE \
  PIP_EXTRA_INDEX_URL \
  PIP_INDEX_URL \
  PIP_TRUSTED_HOST \
  UV_EXTRA_INDEX_URL \
  UV_INDEX_URL; do
  if [[ -n "${!ambient_name-}" ]]; then
    fail "ambient-environment:$ambient_name"
  fi
done
if [[ "$failures" -ne 0 ]]; then
  not_run "all-gates:unsafe-ambient-environment"
  exit 1
fi
pass "ambient-environment:sealed"

run_gate \
  "inactive-policy" \
  /bin/bash "$REPO_ROOT/infra/deployment/tests/check-inactive-policy.sh" ||
  {
    not_run "pinned-tooling:static-policy-failed"
    exit 1
  }

python312="$(command -v python3.12 || true)"
if [[ -z "$python312" ]]; then
  blocked "python:3.12-unavailable"
  not_run "ansible:pinned-environment"
  not_run "molecule:pinned-environment"
  not_run "linux:systemd-and-quadlet"
  not_run "prometheus:rule-tests"
  exit 2
fi
if [[ "$("$python312" -c 'import platform; print(platform.python_version())')" != 3.12.* ]]; then
  fail "python:expected-3.12"
  not_run "pinned-tooling:invalid-python"
  exit 1
fi
pass "python:3.12"

if [[ -d /private/tmp ]]; then
  readonly TASK_TMP_PARENT=/private/tmp
else
  readonly TASK_TMP_PARENT=/tmp
fi
tmp_root="$(mktemp -d "$TASK_TMP_PARENT/alpha-task9-check.XXXXXX")"
readonly tmp_root
venv="$tmp_root/venv"
readonly venv
readonly VERIFIER_PATH="$venv/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

run_gate "python:venv-create" "$python312" -m venv "$venv" ||
  {
    not_run "pinned-tooling:venv-create-failed"
    exit 1
  }

run_gate \
  "python:full-hash-lock-install" \
  env PIP_DISABLE_PIP_VERSION_CHECK=1 \
  "$venv/bin/python" -m pip install \
  --require-hashes \
  --only-binary=:all: \
  --no-input \
  --requirement "$LOCK" ||
  {
    not_run "ansible:lock-install-failed"
    not_run "molecule:lock-install-failed"
    exit 1
  }

verify_versions() {
  "$venv/bin/python" -c '
from importlib.metadata import version
expected = {
    "ansible-core": "2.21.2",
    "ansible-lint": "26.6.0",
    "docker": "7.2.0",
    "molecule": "26.6.0",
    "molecule-plugins": "26.7.15",
}
actual = {name: version(name) for name in expected}
assert actual == expected, (actual, expected)
'
}
run_gate "tooling:exact-versions" verify_versions || exit 1
run_gate "tooling:dependency-consistency" "$venv/bin/python" -m pip check || exit 1

export PATH="$VERIFIER_PATH"
mkdir -p \
  "$tmp_root/ansible_async" \
  "$tmp_root/ansible_collections" \
  "$tmp_root/ansible_home" \
  "$tmp_root/ansible_local"
export ANSIBLE_ASYNC_DIR="$tmp_root/ansible_async"
export ANSIBLE_COLLECTIONS_PATH="$tmp_root/ansible_collections"
export ANSIBLE_CONFIG="$ANSIBLE_DIR/ansible.cfg"
export ANSIBLE_HOME="$tmp_root/ansible_home"
export ANSIBLE_LOCAL_TEMP="$tmp_root/ansible_local"
export ANSIBLE_ROLES_PATH="$ANSIBLE_DIR/roles"
pass "tooling:path-sealed"
pass "ansible:paths-sealed"

run_gate \
  "ansible:exact-collections-install" \
  "$venv/bin/ansible-galaxy" collection install \
  --requirements-file "$COLLECTION_REQUIREMENTS" \
  --collections-path "$ANSIBLE_COLLECTIONS_PATH" \
  --no-deps ||
  {
    not_run "molecule:collections-install-failed"
    exit 1
  }

verify_collection_versions() {
  "$venv/bin/python" - "$ANSIBLE_COLLECTIONS_PATH" <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
expected = {
    "ansible.posix": "2.2.2",
    "community.docker": "5.2.1",
    "community.library_inventory_filtering_v1": "1.1.5",
}
installed = {}
for qualified_name, expected_version in expected.items():
    namespace, collection = qualified_name.split(".", maxsplit=1)
    manifest_path = root / namespace / collection / "MANIFEST.json"
    with manifest_path.open(encoding="utf-8") as manifest_file:
        info = json.load(manifest_file)["collection_info"]
    actual_identity = f"{info['namespace']}.{info['name']}"
    installed[actual_identity] = info["version"]
    assert actual_identity == qualified_name, (actual_identity, qualified_name)
    assert info["version"] == expected_version, (info["version"], expected_version)
assert installed == expected, (installed, expected)
PY
}
run_gate "ansible:exact-collection-versions" verify_collection_versions ||
  {
    not_run "molecule:collection-version-mismatch"
    exit 1
  }

(
  cd "$ANSIBLE_DIR"
  run_gate "yaml:lint" "$venv/bin/yamllint" . &&
    run_gate "ansible:lint" ansible-lint --offline . &&
    run_gate \
      "ansible:inventory-parse-example-only" \
      "$venv/bin/ansible-inventory" -i "$INVENTORY" --list &&
    run_gate \
      "ansible:syntax-example-only" \
      "$venv/bin/ansible-playbook" -i "$INVENTORY" "$PLAYBOOK" --syntax-check
) || {
  not_run "molecule:static-ansible-gate-failed"
  exit 1
}

if ! command -v docker >/dev/null 2>&1; then
  blocked "docker:cli-unavailable"
  not_run "molecule:docker-unavailable"
  not_run "linux:docker-unavailable"
  not_run "prometheus:docker-unavailable"
  exit 2
fi
if ! docker info >/dev/null 2>&1; then
  blocked "docker:daemon-unavailable"
  not_run "molecule:docker-unavailable"
  not_run "linux:docker-unavailable"
  not_run "prometheus:docker-unavailable"
  exit 2
fi
pass "docker:daemon-available"

if ! docker image inspect "$UBUNTU_IMAGE" >/dev/null 2>&1; then
  run_gate "docker:pull-pinned-ubuntu-verifier" docker pull "$UBUNTU_IMAGE" ||
    {
      blocked "docker:pinned-ubuntu-verifier-unavailable"
      not_run "molecule:ubuntu-verifier-unavailable"
      not_run "linux:ubuntu-verifier-unavailable"
      not_run "prometheus:prior-verifier-blocked"
      exit 2
    }
else
  pass "docker:pinned-ubuntu-verifier-present"
fi

run_molecule_gates() {
  cd "$ANSIBLE_DIR"
  export MOLECULE_EPHEMERAL_DIRECTORY="$tmp_root/molecule"
  run_gate "molecule:syntax" env -u HOME molecule syntax || return 1
  run_gate "molecule:create" env -u HOME molecule create || return 1
  molecule_created=1
  run_gate "molecule:prepare" env -u HOME molecule prepare || return 1
  run_gate "ansible:first-converge" env -u HOME molecule converge || return 1
  run_gate "ansible:check-mode" env -u HOME molecule check || return 1
  run_gate "molecule:idempotence" env -u HOME molecule idempotence || return 1
  run_gate "molecule:verify" env -u HOME molecule verify || return 1
  run_gate "molecule:destroy" env -u HOME molecule destroy || return 1
  molecule_created=0
}
run_molecule_gates || {
  not_run "molecule:remaining-dependent-gates"
  exit 1
}

run_gate \
  "linux-verifier:build" \
  docker build \
  --pull=false \
  --file "$REPO_ROOT/infra/deployment/tests/Dockerfile.linux-verifier" \
  --tag "$LINUX_VERIFIER_IMAGE" \
  "$REPO_ROOT" ||
  {
    not_run "linux:systemd-and-quadlet"
    exit 1
  }
linux_image_created=1

run_gate \
  "linux-verifier:systemd-and-quadlet" \
  docker run \
  --rm \
  --read-only \
  --tmpfs /opt:rw,exec,nosuid,nodev \
  --tmpfs /tmp:rw,noexec,nosuid,nodev \
  --tmpfs /run:rw,noexec,nosuid,nodev \
  --volume "$REPO_ROOT:/workspace:ro" \
  "$LINUX_VERIFIER_IMAGE" ||
  exit 1

if ! docker image inspect "$PROMETHEUS_IMAGE" >/dev/null 2>&1; then
  run_gate "docker:pull-pinned-promtool-verifier" docker pull "$PROMETHEUS_IMAGE" ||
    {
      blocked "prometheus:pinned-verifier-unavailable"
      not_run "prometheus:rule-syntax-and-tests"
      exit 2
    }
else
  pass "docker:pinned-promtool-verifier-present"
fi

run_gate \
  "prometheus:rule-syntax" \
  docker run \
  --rm \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,nodev \
  --volume "$REPO_ROOT/infra/monitoring/alerts:/work:ro" \
  --workdir /work \
  --entrypoint /bin/promtool \
  "$PROMETHEUS_IMAGE" \
  check rules foundations.yml ||
  {
    not_run "prometheus:rule-tests"
    exit 1
  }

run_gate \
  "prometheus:rule-tests" \
  docker run \
  --rm \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,nodev \
  --volume "$REPO_ROOT/infra/monitoring/alerts:/work:ro" \
  --workdir /work \
  --entrypoint /bin/promtool \
  "$PROMETHEUS_IMAGE" \
  test rules foundations.test.yml ||
  exit 1

if [[ "$failures" -ne 0 ]]; then
  printf 'FAIL task9-verification failures=%s blocked=%s\n' \
    "$failures" "$blocked_count" >&2
  exit 1
fi
if [[ "$blocked_count" -ne 0 ]]; then
  printf 'BLOCKED task9-verification failures=0 blocked=%s\n' \
    "$blocked_count" >&2
  exit 2
fi
printf 'PASS task9-verification failures=0 blocked=0\n'
