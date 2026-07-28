#!/usr/bin/env bash
# shellcheck disable=SC2016
# Literal contract probes intentionally match unexpanded $instance/$run_id text.
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
REPO_ROOT="$(
  CDPATH='' builtin cd -- "$SCRIPT_DIR/../../.." &&
    builtin pwd -P
)"
readonly REPO_ROOT

failures=0

pass() {
  printf 'PASS %s\n' "$1"
}

fail() {
  printf 'FAIL %s\n' "$1" >&2
  failures=$((failures + 1))
}

require_file() {
  local relative=$1
  if [[ -f "$REPO_ROOT/$relative" ]]; then
    pass "file:$relative"
  else
    fail "missing-file:$relative"
    return 1
  fi
}

assert_absent_pattern() {
  local label=$1
  local pattern=$2
  shift 2
  local -a existing=()
  local path
  for path in "$@"; do
    [[ -e "$path" ]] && existing+=("$path")
  done
  if [[ "${#existing[@]}" -eq 0 ]]; then
    fail "$label:no-inputs"
  elif rg -n -i -- "$pattern" "${existing[@]}" >/dev/null; then
    fail "$label"
  else
    pass "$label"
  fi
}

assert_present_pattern() {
  local label=$1
  local pattern=$2
  local path=$3
  if [[ ! -f "$path" ]]; then
    fail "$label:missing-input"
  elif rg -n -- "$pattern" "$path" >/dev/null; then
    pass "$label"
  else
    fail "$label"
  fi
}

readonly EXPECTED_SERVICES="$SCRIPT_DIR/fixtures/allowed-services.txt"
readonly FORBIDDEN_INSTANCES="$SCRIPT_DIR/fixtures/forbidden-instances.txt"
readonly GROUP_VARS="$REPO_ROOT/infra/ansible/group_vars/all.yml"
readonly INVENTORY="$REPO_ROOT/infra/ansible/inventory/example/hosts.yml"
readonly INSTANCE_VALIDATOR="$REPO_ROOT/infra/systemd/validate-instance.sh"
readonly SYSTEMD_UNIT="$REPO_ROOT/infra/systemd/hl-service@.service"
readonly ALERT_RULES="$REPO_ROOT/infra/monitoring/alerts/foundations.yml"
readonly ALERT_TESTS="$REPO_ROOT/infra/monitoring/alerts/foundations.test.yml"
readonly REQUIREMENTS_LOCK="$REPO_ROOT/infra/ansible/requirements.lock"
readonly REQUIREMENTS_INPUT="$REPO_ROOT/infra/ansible/requirements.in"
readonly COLLECTION_REQUIREMENTS="$REPO_ROOT/infra/ansible/collections/requirements.yml"
readonly MOLECULE_CONFIG="$REPO_ROOT/infra/ansible/molecule/default/molecule.yml"
readonly MOLECULE_DOCKERFILE="$REPO_ROOT/infra/ansible/molecule/default/Dockerfile.j2"
readonly COMMON_TASKS="$REPO_ROOT/infra/ansible/roles/common/tasks/main.yml"
readonly SERVICE_TASKS="$REPO_ROOT/infra/ansible/roles/alpha_service/tasks/main.yml"
readonly BACKUP_POLICY="$REPO_ROOT/infra/backup/README.md"
readonly ANSIBLE_HARNESS="$REPO_ROOT/infra/ansible/tests/check.sh"

require_file "infra/ansible/requirements.in" || true
require_file "infra/ansible/requirements.lock" || true
require_file "infra/ansible/ansible.cfg" || true
require_file "infra/ansible/collections/requirements.yml" || true
require_file "infra/ansible/tests/check.sh" || true
require_file "infra/ansible/inventory/example/hosts.yml" || true
require_file "infra/ansible/group_vars/all.yml" || true
require_file "infra/ansible/roles/common/tasks/main.yml" || true
require_file "infra/ansible/roles/alpha_service/tasks/main.yml" || true
require_file "infra/ansible/molecule/default/Dockerfile.j2" || true
require_file "infra/ansible/molecule/default/prepare.yml" || true
require_file "infra/deployment/tests/Dockerfile.linux-verifier" || true
require_file "infra/deployment/tests/verify-linux.sh" || true
require_file "infra/systemd/validate-instance.sh" || true
require_file "infra/systemd/hl-service@.service" || true
require_file "infra/systemd/hl-service.env.example" || true
require_file "infra/podman/quadlet/nats.container" || true
require_file "infra/podman/quadlet/postgresql.container" || true
require_file "infra/podman/quadlet/clickhouse.container" || true
require_file "infra/podman/quadlet/minio.container" || true
require_file "infra/monitoring/alerts/foundations.yml" || true
require_file "infra/monitoring/alerts/foundations.test.yml" || true
require_file "infra/backup/README.md" || true
require_file "docs/runbooks/service-lifecycle.md" || true

if diff -u - "$REQUIREMENTS_INPUT" >/dev/null <<'EXPECTED_REQUIREMENTS'
ansible-core==2.21.2
ansible-lint==26.6.0
docker==7.2.0
molecule==26.6.0
molecule-plugins[docker]==26.7.15
EXPECTED_REQUIREMENTS
then
  pass "requirements:exact-direct-pins"
else
  fail "requirements:exact-direct-pins"
fi

if diff -u - "$COLLECTION_REQUIREMENTS" >/dev/null <<'EXPECTED_COLLECTIONS'
---
collections:
  - name: ansible.posix
    version: 2.2.2
  - name: community.docker
    version: 5.2.1
  - name: community.library_inventory_filtering_v1
    version: 1.1.5
EXPECTED_COLLECTIONS
then
  pass "collections:exact-pins"
else
  fail "collections:exact-pins"
fi

if [[ -f "$REQUIREMENTS_LOCK" ]]; then
  lock_status=0
  for requirement in \
    'ansible-core==2.21.2' \
    'ansible-lint==26.6.0' \
    'docker==7.2.0' \
    'molecule==26.6.0' \
    'molecule-plugins==26.7.15'; do
    rg -F "$requirement" "$REQUIREMENTS_LOCK" >/dev/null || lock_status=1
  done
  rg -- '--hash=sha256:' "$REQUIREMENTS_LOCK" >/dev/null || lock_status=1
  if [[ "$lock_status" -eq 0 ]]; then
    pass "requirements:fully-hashed-direct-pins"
  else
    fail "requirements:fully-hashed-direct-pins"
  fi
fi

if [[ -f "$GROUP_VARS" ]]; then
  actual_services="$(
    sed -n '/^alpha_v1_services:/,/^[^[:space:]-]/s/^[[:space:]]*- //p' \
      "$GROUP_VARS" |
      LC_ALL=C sort
  )"
  if diff -u "$EXPECTED_SERVICES" - <<<"$actual_services" >/dev/null; then
    pass "services:exact-five"
  else
    fail "services:exact-five"
  fi
fi

if [[ -x "$INSTANCE_VALIDATOR" ]]; then
  validator_status=0
  while IFS= read -r instance; do
    "$INSTANCE_VALIDATOR" "$instance" >/dev/null 2>&1 || validator_status=1
  done <"$EXPECTED_SERVICES"
  while IFS= read -r instance; do
    if "$INSTANCE_VALIDATOR" "$instance" >/dev/null 2>&1; then
      validator_status=1
    fi
  done <"$FORBIDDEN_INSTANCES"
  if [[ "$validator_status" -eq 0 ]]; then
    pass "services:validator-behavior"
  else
    fail "services:validator-behavior"
  fi
fi

if [[ -f "$INVENTORY" ]]; then
  inventory_status=0
  rg -n -i \
    'localhost|(^|[^0-9])([0-9]{1,3}\.){3}[0-9]{1,3}([^0-9]|$)|::|ansible_connection:[[:space:]]*local|ansible_(user|ssh_private_key_file|password)' \
    "$INVENTORY" >/dev/null &&
    inventory_status=1
  inventory_hosts="$(
    awk '
      /^[[:space:]]+hosts:[[:space:]]*$/ {
        in_hosts = 1
        hosts_indent = match($0, /[^ ]/) - 1
        next
      }
      in_hosts {
        indent = match($0, /[^ ]/) - 1
        if ($0 !~ /^[[:space:]]*$/ && indent <= hosts_indent) {
          in_hosts = 0
          next
        }
        if ($0 ~ /^[[:space:]]+[A-Za-z0-9.-]+:[[:space:]]*$/) {
          key = $0
          sub(/^[[:space:]]+/, "", key)
          sub(/:[[:space:]]*$/, "", key)
          print key
        }
      }
    ' "$INVENTORY"
  )"
  [[ -n "$inventory_hosts" ]] || inventory_status=1
  while IFS= read -r inventory_host; do
    [[ "$inventory_host" == *.invalid ]] || inventory_status=1
  done <<<"$inventory_hosts"
  if [[ "$inventory_status" -eq 0 ]]; then
    pass "inventory:documentation-hosts-only"
  else
    fail "inventory:documentation-hosts-only"
  fi
fi

ansible_tasks=("$REPO_ROOT/infra/ansible/roles/.missing-task-input")
while IFS= read -r task_file; do
  ansible_tasks[${#ansible_tasks[@]}]="$task_file"
done < <(
  find "$REPO_ROOT/infra/ansible/roles" -type f -path '*/tasks/*.yml' -print 2>/dev/null |
    LC_ALL=C sort
)
assert_absent_pattern \
  "ansible:no-activation-or-host-mutation" \
  'ansible\.builtin\.(service|systemd|systemd_service)|systemctl|state:[[:space:]]*(started|stopped|restarted)|enabled:[[:space:]]*true|daemon_reload|ufw[[:space:]]+(enable|reload)|timedatectl|hwclock|enable-linger|loginctl|podman[[:space:]]+(pull|volume|secret)|/opt/hyperliquid-alpha-desk/bin/' \
  "${ansible_tasks[@]}"
assert_absent_pattern \
  "ansible:no-package-holds-or-downgrades" \
  'apt-mark[[:space:]]+hold|allow_downgrade:[[:space:]]*true' \
  "${ansible_tasks[@]}"
assert_present_pattern \
  "ansible:package-policy-rc-d" \
  'policy_rc_d:[[:space:]]*101' \
  "$COMMON_TASKS"

ssh_review_status=0
grep -F '/etc/ssh/sshd_config.d/00-alpha-desk-stage0.conf' \
  "$COMMON_TASKS" >/dev/null ||
  ssh_review_status=1
rg -n '/etc/ssh/sshd_config.d/99-alpha-desk-stage0.conf' \
  "$COMMON_TASKS" >/dev/null &&
  ssh_review_status=1
for effective_assertion in \
  'permitrootlogin no' \
  'passwordauthentication no' \
  'kbdinteractiveauthentication no' \
  'pubkeyauthentication yes' \
  'x11forwarding no' \
  'allowtcpforwarding no'; do
  grep -F "$effective_assertion" \
    "$REPO_ROOT/infra/ansible/molecule/default/verify.yml" >/dev/null ||
    ssh_review_status=1
done
if [[ "$ssh_review_status" -eq 0 ]]; then
  pass "review:ssh-effective-precedence"
else
  fail "review:ssh-effective-precedence"
fi

validator_review_status=0
grep -Fx \
  'ExecCondition=/usr/libexec/hyperliquid-alpha-desk/validate-instance %i' \
  "$SYSTEMD_UNIT" >/dev/null ||
  validator_review_status=1
grep -F '/usr/libexec/hyperliquid-alpha-desk/validate-instance' \
  "$SERVICE_TASKS" >/dev/null ||
  validator_review_status=1
for allowed_instance in \
  hl-analytics \
  hl-api \
  hl-capture \
  hl-core \
  hl-research; do
  grep -Fx "  $allowed_instance" \
    "$REPO_ROOT/infra/deployment/tests/verify-linux.sh" >/dev/null ||
    validator_review_status=1
done
grep -F 'hl-service@$instance.service' \
  "$REPO_ROOT/infra/deployment/tests/verify-linux.sh" >/dev/null ||
  validator_review_status=1
grep -F 'hl-service@hl-exec.service' \
  "$REPO_ROOT/infra/deployment/tests/verify-linux.sh" >/dev/null ||
  validator_review_status=1
if [[ "$validator_review_status" -eq 0 ]]; then
  pass "review:unit-instance-validator"
else
  fail "review:unit-instance-validator"
fi

package_review_status=0
expected_package_specs="$(command cat <<'EXPECTED_PACKAGE_SPECS'
chrony=4.5-1ubuntu4.2
logrotate=3.21.0-2build1
openssh-server=1:9.6p1-3ubuntu13.18
sudo=1.9.15p5-3ubuntu5.24.04.2
ufw=0.36.2-6
unattended-upgrades=2.9.1+nmu4ubuntu1
EXPECTED_PACKAGE_SPECS
)"
actual_package_specs="$(
  sed -n \
    '/^alpha_stage0_packages:$/,/^[^[:space:]-]/s/^[[:space:]]*- //p' \
    "$GROUP_VARS"
)"
[[ "$actual_package_specs" == "$expected_package_specs" ]] ||
  package_review_status=1
for pin_contract in \
  '/etc/apt/preferences.d/alpha-desk-stage0' \
  'Pin-Priority: 1000' \
  'allow_downgrade: false' \
  'allow_change_held_packages: false' \
  'check_mode: false'; do
  grep -F "$pin_contract" "$COMMON_TASKS" >/dev/null ||
    package_review_status=1
done
rg -n 'Pin-Priority:[[:space:]]*(100[1-9]|10[1-9][0-9]|1[1-9][0-9]{2,})' \
  "$COMMON_TASKS" >/dev/null &&
  package_review_status=1
rg -n 'cache_valid_time:' "$COMMON_TASKS" >/dev/null &&
  package_review_status=1
if [[ "$package_review_status" -eq 0 ]]; then
  pass "review:noble-package-pinning"
else
  fail "review:noble-package-pinning"
fi

ufw_review_status=0
grep -F '/etc/default/ufw' "$COMMON_TASKS" >/dev/null ||
  ufw_review_status=1
for ufw_default in \
  'DEFAULT_INPUT_POLICY="DROP"' \
  'DEFAULT_OUTPUT_POLICY="ACCEPT"' \
  'DEFAULT_FORWARD_POLICY="DROP"'; do
  grep -F "$ufw_default" "$COMMON_TASKS" >/dev/null ||
    ufw_review_status=1
done
if [[ "$ufw_review_status" -eq 0 ]]; then
  pass "review:ufw-effective-defaults"
else
  fail "review:ufw-effective-defaults"
fi

backup_review_status=0
for backup_contract in \
  'Recent compatible RocksDB checkpoints' \
  'At least one recent compatible checkpoint' \
  'current plus checkpoints' \
  'secondary operator-controlled site' \
  'block height' \
  'canonical archive manifest hash' \
  'schema versions' \
  'state hash' \
  'nearest compatible checkpoint' \
  'subsequent events'; do
  grep -F "$backup_contract" "$BACKUP_POLICY" >/dev/null ||
    backup_review_status=1
done
if [[ "$backup_review_status" -eq 0 ]]; then
  pass "review:rocksdb-backup-policy"
else
  fail "review:rocksdb-backup-policy"
fi

concurrency_review_status=0
for molecule_contract in \
  'alpha-stage0-noble-${ALPHA_TASK9_RUN_ID}' \
  'alpha-desk-task9-molecule:${ALPHA_TASK9_RUN_ID}'; do
  grep -F "$molecule_contract" "$MOLECULE_CONFIG" >/dev/null ||
    concurrency_review_status=1
done
for harness_contract in \
  'ALPHA_TASK9_RUN_ID="$run_id"' \
  'MOLECULE_IMAGE="molecule_local/alpha-desk-task9-molecule:${run_id}"' \
  'alpha-desk-task9-linux-verifier:${run_id}' \
  'docker image rm "$MOLECULE_IMAGE"' \
  'docker image rm "$LINUX_VERIFIER_IMAGE"'; do
  grep -F "$harness_contract" "$ANSIBLE_HARNESS" >/dev/null ||
    concurrency_review_status=1
done
rg -n -- '--filter label=org[.]alpha-desk[.]task9-verifier=molecule' \
  "$ANSIBLE_HARNESS" >/dev/null &&
  concurrency_review_status=1
if [[ "$concurrency_review_status" -eq 0 ]]; then
  pass "review:concurrent-owned-cleanup"
else
  fail "review:concurrent-owned-cleanup"
fi

if [[ -f "$SYSTEMD_UNIT" ]]; then
  unit_status=0
  for directive in \
    '[Unit]' \
    'After=network-online.target' \
    'Wants=network-online.target' \
    'User=hl-%i' \
    'Group=hl-%i' \
    'ExecCondition=/usr/libexec/hyperliquid-alpha-desk/validate-instance %i' \
    'ExecStart=/opt/hyperliquid-alpha-desk/bin/%i --config /etc/hyperliquid-alpha-desk/%i.toml' \
    'Restart=on-failure' \
    'RestartSec=2s' \
    'EnvironmentFile=-/etc/hyperliquid-alpha-desk/%i.env' \
    'NoNewPrivileges=true' \
    'PrivateTmp=true' \
    'ProtectSystem=strict' \
    'ProtectHome=true' \
    'ProtectKernelTunables=true' \
    'ProtectKernelModules=true' \
    'ProtectControlGroups=true' \
    'RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6' \
    'LockPersonality=true' \
    'MemoryDenyWriteExecute=true' \
    'CapabilityBoundingSet=' \
    'AmbientCapabilities=' \
    'ReadWritePaths=/var/lib/hyperliquid-alpha-desk/%i /var/log/hyperliquid-alpha-desk/%i' \
    'LimitNOFILE=1048576'; do
    grep -Fx "$directive" "$SYSTEMD_UNIT" >/dev/null || unit_status=1
  done
  grep -Fx '[Install]' "$SYSTEMD_UNIT" >/dev/null && unit_status=1
  if [[ "$unit_status" -eq 0 ]]; then
    pass "systemd:hardened-inactive-template"
  else
    fail "systemd:hardened-inactive-template"
  fi
fi

check_quadlet() {
  local name=$1
  local expected_image=$2
  local expected_volume=$3
  local path="$REPO_ROOT/infra/podman/quadlet/$name.container"
  [[ -f "$path" ]] || return 0
  local status=0
  grep -Fx "Image=$expected_image" "$path" >/dev/null || status=1
  for directive in \
    'Pull=never' \
    'ReadOnly=true' \
    'NoNewPrivileges=true' \
    'DropCapability=all'; do
    grep -Fx "$directive" "$path" >/dev/null || status=1
  done
  rg -n '^\[Install\]$|^(User|Group|Secret|AutoUpdate)=' "$path" >/dev/null &&
    status=1
  volume_count="$(rg -c '^Volume=' "$path" || true)"
  [[ "$volume_count" -eq 1 ]] || status=1
  grep -Fx "Volume=$expected_volume" "$path" >/dev/null || status=1
  rg -n '^Volume=(/|~|\.\.?/)|^Volume=[^:[:space:]]+:[^/]' "$path" >/dev/null &&
    status=1
  if rg -n '^PublishPort=' "$path" >/dev/null; then
    while IFS= read -r published_port; do
      [[ "$published_port" =~ ^PublishPort=127\.0\.0\.1:[0-9]+:[0-9]+(/(tcp|udp))?$ ]] ||
        status=1
    done < <(rg '^PublishPort=' "$path")
  fi
  if [[ "$status" -eq 0 ]]; then
    pass "quadlet:$name:inactive-policy"
  else
    fail "quadlet:$name:inactive-policy"
  fi
}

check_quadlet \
  nats \
  'docker.io/library/nats:2.14.3-alpine@sha256:c11af972c99ae542de8925e6a7d9c533aa1eb039660420d2074beed6089b3bf0' \
  'alpha-desk-nats-data:/data'
check_quadlet \
  postgresql \
  'docker.io/library/postgres:18.4-alpine3.24@sha256:9a8afca54e7861fd90fab5fdf4c42477a6b1cb7d293595148e674e0a3181de15' \
  'alpha-desk-postgresql-data:/var/lib/postgresql'
check_quadlet \
  clickhouse \
  'docker.io/clickhouse/clickhouse-server:26.3.17.56-alpine@sha256:1d8f5b3febaf81be475cc0fe8bf71acdc09e90ffcf1d8c8e291674d4f13c29bd' \
  'alpha-desk-clickhouse-data:/var/lib/clickhouse'
check_quadlet \
  minio \
  'quay.io/minio/minio:RELEASE.2025-09-07T16-13-09Z@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e' \
  'alpha-desk-minio-data:/data'

if [[ -f "$MOLECULE_CONFIG" ]]; then
  molecule_status=0
  grep -Fx \
    '    image: alpha-desk-task9-molecule:${ALPHA_TASK9_RUN_ID}' \
    "$MOLECULE_CONFIG" >/dev/null ||
    molecule_status=1
  grep -Fx '  enabled: false' "$MOLECULE_CONFIG" >/dev/null ||
    molecule_status=1
  grep -Fx '    privileged: false' "$MOLECULE_CONFIG" >/dev/null ||
    molecule_status=1
  for action in syntax create prepare converge idempotence verify destroy; do
    grep -Fx "    - $action" "$MOLECULE_CONFIG" >/dev/null ||
      molecule_status=1
  done
  for sequence in \
    check_sequence \
    converge_sequence \
    create_sequence \
    destroy_sequence \
    idempotence_sequence \
    prepare_sequence \
    syntax_sequence \
    verify_sequence; do
    grep -Fx "  $sequence:" "$MOLECULE_CONFIG" >/dev/null ||
      molecule_status=1
  done
  rg -n 'privileged:[[:space:]]*true|cap_add:|volumes:|/var/run/docker\.sock' \
    "$MOLECULE_CONFIG" >/dev/null &&
    molecule_status=1
  actual_test_sequence="$(
    sed -n \
      '/^  test_sequence:$/,/^[^[:space:]]/s/^    - //p' \
      "$MOLECULE_CONFIG"
  )"
  expected_test_sequence="$(command cat <<'EXPECTED_SEQUENCE'
syntax
create
prepare
converge
check
idempotence
verify
destroy
EXPECTED_SEQUENCE
)"
  if [[ "$actual_test_sequence" != "$expected_test_sequence" ]]; then
    molecule_status=1
  fi
  if [[ "$molecule_status" -eq 0 ]]; then
    pass "molecule:pinned-unprivileged-scenario"
  else
    fail "molecule:pinned-unprivileged-scenario"
  fi
fi

shared_vars_status=0
for molecule_playbook in converge verify; do
  grep -Fx '  vars_files:' \
    "$REPO_ROOT/infra/ansible/molecule/default/$molecule_playbook.yml" \
    >/dev/null ||
    shared_vars_status=1
  grep -Fx '    - ../../group_vars/all.yml' \
    "$REPO_ROOT/infra/ansible/molecule/default/$molecule_playbook.yml" \
    >/dev/null ||
    shared_vars_status=1
done
if [[ "$shared_vars_status" -eq 0 ]]; then
  pass "molecule:shared-vars-single-source"
else
  fail "molecule:shared-vars-single-source"
fi

if rg -n '^export HOME=' "$REPO_ROOT/infra/ansible/tests/check.sh" >/dev/null; then
  fail "harness:home-not-repurposed"
else
  pass "harness:home-not-repurposed"
fi

linux_fixture_status=0
grep -F '/opt/hyperliquid-alpha-desk/bin/i' \
  "$REPO_ROOT/infra/deployment/tests/verify-linux.sh" >/dev/null ||
  linux_fixture_status=1
grep -F '/opt/hyperliquid-alpha-desk/bin/$instance' \
  "$REPO_ROOT/infra/deployment/tests/verify-linux.sh" >/dev/null ||
  linux_fixture_status=1
grep -Fx "  --tmpfs /opt:rw,exec,nosuid,nodev \\" \
  "$REPO_ROOT/infra/ansible/tests/check.sh" >/dev/null ||
  linux_fixture_status=1
grep -Fx \
  "  --tmpfs /usr/libexec/hyperliquid-alpha-desk:rw,exec,nosuid,nodev \\" \
  "$REPO_ROOT/infra/ansible/tests/check.sh" >/dev/null ||
  linux_fixture_status=1
rg -n -- '--tmpfs /opt:[^[:space:]]*noexec' \
  "$REPO_ROOT/infra/ansible/tests/check.sh" >/dev/null &&
  linux_fixture_status=1
if [[ "$linux_fixture_status" -eq 0 ]]; then
  pass "linux-verifier:executable-materialized-fixtures"
else
  fail "linux-verifier:executable-materialized-fixtures"
fi

promtool_tmpfs_count="$(
  grep -Fc "  --tmpfs /tmp:rw,noexec,nosuid,nodev \\" \
    "$REPO_ROOT/infra/ansible/tests/check.sh" ||
    true
)"
if [[ "$promtool_tmpfs_count" -ge 3 ]]; then
  pass "promtool:hardened-writable-tmpfs"
else
  fail "promtool:hardened-writable-tmpfs"
fi

if [[ -f "$MOLECULE_DOCKERFILE" ]]; then
  molecule_image_status=0
  grep -Fx \
    'FROM ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90' \
    "$MOLECULE_DOCKERFILE" >/dev/null ||
    molecule_image_status=1
  grep -Fx 'LABEL org.alpha-desk.task9-verifier="molecule"' \
    "$MOLECULE_DOCKERFILE" >/dev/null ||
    molecule_image_status=1
  grep -F "printf '%s\\n' '#!/bin/sh' 'exit 101' > /usr/sbin/policy-rc.d" \
    "$MOLECULE_DOCKERFILE" >/dev/null ||
    molecule_image_status=1
  grep -F 'python3 python3-apt sudo ca-certificates' \
    "$MOLECULE_DOCKERFILE" >/dev/null ||
    molecule_image_status=1
  rg -n 'systemd|privileged|docker[.]sock' "$MOLECULE_DOCKERFILE" >/dev/null &&
    molecule_image_status=1
  if [[ "$molecule_image_status" -eq 0 ]]; then
    pass "molecule:pinned-python-bootstrap-image"
  else
    fail "molecule:pinned-python-bootstrap-image"
  fi
fi

if [[ -f "$ALERT_RULES" ]]; then
  alert_status=0
  alert_count="$(rg -c '^[[:space:]]+- alert:' "$ALERT_RULES" || true)"
  for field in \
    'for:' \
    'severity:' \
    'team:' \
    'zone:' \
    'summary:' \
    'description:' \
    'runbook_url:'; do
    count="$(rg -c "^[[:space:]]+$field" "$ALERT_RULES" || true)"
    [[ "$count" -eq "$alert_count" ]] || alert_status=1
  done
  rg -n \
    'alpha_desk_health_assessments_total\{[^}]*state="red"[^}]*\}[[:space:]]*(==|>|>=)' \
    "$ALERT_RULES" >/dev/null &&
    alert_status=1
  if rg -n 'alpha_desk_health_assessments_total' "$ALERT_RULES" >/dev/null; then
    rg -n 'increase\(alpha_desk_health_assessments_total\{' \
      "$ALERT_RULES" >/dev/null ||
      alert_status=1
  fi
  if [[ "$alert_status" -eq 0 && "$alert_count" -gt 0 ]]; then
    pass "alerts:truthful-static-contract"
  else
    fail "alerts:truthful-static-contract"
  fi
fi

if [[ -f "$ALERT_TESTS" ]]; then
  alert_test_status=0
  for scenario in firing nonfiring missing recovery; do
    rg -n -i "$scenario" "$ALERT_TESTS" >/dev/null || alert_test_status=1
  done
  if [[ "$alert_test_status" -eq 0 ]]; then
    pass "alerts:semantic-test-cases-declared"
  else
    fail "alerts:semantic-test-cases-declared"
  fi
fi

assert_absent_pattern \
  "quadlet:no-live-operations" \
  'podman[[:space:]]+(pull|volume[[:space:]]+create|secret[[:space:]]+create)|systemctl([[:space:]]+--user)?[[:space:]]+(start|enable|daemon-reload)|loginctl[[:space:]]+enable-linger' \
  "$REPO_ROOT/infra/podman" \
  "$REPO_ROOT/infra/ansible"

if [[ "$failures" -ne 0 ]]; then
  printf 'FAIL inactive-policy failures=%s\n' "$failures" >&2
  exit 1
fi

printf 'PASS inactive-policy failures=0\n'
