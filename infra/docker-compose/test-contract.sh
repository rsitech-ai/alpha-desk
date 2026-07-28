#!/usr/bin/env bash
set -euo pipefail

script_dir="$(
  CDPATH='' builtin cd -- "$(command dirname -- "${BASH_SOURCE[0]}")" &&
    builtin pwd -P
)"
repo_root="$(
  CDPATH='' builtin cd -- "$script_dir/../.." &&
    builtin pwd -P
)"
compose_file="$script_dir/compose.yaml"
lock_file="$script_dir/images.lock"
wait_script="$repo_root/tools/ci/wait-for-dev-stack.sh"

for path in "$compose_file" "$lock_file" "$wait_script"; do
  [[ -f "$path" ]] || {
    printf 'dev-stack-contract:error missing %s\n' "$path" >&2
    exit 1
  }
done

for command_name in docker jq just; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'dev-stack-contract:error missing command %s\n' "$command_name" >&2
    exit 1
  }
done

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/alpha-desk-dev-stack-test.XXXXXX")"
trap 'rm -rf -- "$tmp_dir"' EXIT

compose_json="$tmp_dir/compose.json"
docker compose -f "$compose_file" config --format json >"$compose_json"

jq -e '
  .schema_version == 1 and
  (.images | type == "array") and
  (.images | length == 9) and
  all(
    .images[];
    . as $image |
    ($image.service | type == "string" and length > 0) and
    ($image.version | type == "string" and length > 0) and
    ($image.index_digest | test("^sha256:[0-9a-f]{64}$")) and
    ($image.linux_arm64_digest | test("^sha256:[0-9a-f]{64}$")) and
    ($image.compose_ref | endswith("@" + $image.index_digest))
  ) and
  (([.images[].service] | unique | length) == 9)
' "$lock_file" >/dev/null

jq -e '.name == "alpha-desk-dev"' "$compose_json" >/dev/null

jq -e '
  [
    .services
    | to_entries[]
    | .key as $service
    | .value.ports[]?
    | {
        service: $service,
        host_ip: .host_ip,
        published: (.published | tostring),
        target: (.target | tostring)
      }
  ]
  | all(.[]; .host_ip == "127.0.0.1")
  and
  (
    map("\(.service):\(.published):\(.target)") | sort
  ) == (
    [
      "clickhouse:8123:8123",
      "minio:9000:9000",
      "nats:4222:4222",
      "nats:8222:8222",
      "otel-collector:13133:13133",
      "otel-collector:4317:4317",
      "otel-collector:4318:4318",
      "postgres:5432:5432",
      "victoriametrics:8428:8428"
    ] | sort
  )
' "$compose_json" >/dev/null

jq -e '
  [
    .services
    | to_entries[]
    | {
        service: .key,
        image: .value.image,
        security_opt: .value.security_opt,
        cap_drop: .value.cap_drop,
        mem_limit: .value.mem_limit,
        pids_limit: .value.pids_limit,
        read_only: .value.read_only
      }
  ] as $services
  | all(
      $services[];
      (.security_opt | index("no-new-privileges:true")) != null and
      (.cap_drop | index("ALL")) != null and
      (.mem_limit | test("^[1-9][0-9]*$")) and
      (.pids_limit | type == "number" and . > 0) and
      .read_only == true
    )
' "$compose_json" >/dev/null

jq -e '
  . as $root
  | ["nats", "clickhouse", "postgres", "minio", "otel-collector", "victoriametrics"]
  | . as $names
  | all(
      $names[];
      . as $name |
      ($root.services[$name].healthcheck.test | length > 0) and
      ($root.services[$name].restart == "unless-stopped") and
      ($root.services[$name].stop_grace_period | length > 0)
    )
' "$compose_json" >/dev/null

if ! jq -e '
  (.services["nats-init"].restart == "no") and
  (.services["clickhouse-init"].restart == "no") and
  (.services["minio-init"].restart == "no") and
  (.services["nats-init"].depends_on.nats.condition == "service_healthy") and
  (.services["clickhouse-init"].depends_on.clickhouse.condition == "service_healthy") and
  (.services["minio-init"].depends_on.minio.condition == "service_healthy") and
  (.services.victoriametrics.depends_on["otel-collector"].condition == "service_healthy") and
  (.services.victoriametrics.depends_on["nats-init"].condition == "service_completed_successfully") and
  (.services.victoriametrics.depends_on["clickhouse-init"].condition == "service_completed_successfully") and
  (.services.victoriametrics.depends_on["minio-init"].condition == "service_completed_successfully")
' "$compose_json" >/dev/null; then
  printf 'dev-stack-contract:error initializer dependency gate is incomplete\n' >&2
  exit 1
fi

if ! jq -e '
  (.services.clickhouse.environment | has("CLICKHOUSE_DB") | not) and
  (.services["clickhouse-init"].user == "101:101") and
  (.services["clickhouse-init"].entrypoint == ["/usr/bin/clickhouse-client"]) and
  (
    .services["clickhouse-init"].command == [
      "--host",
      "clickhouse",
      "--user",
      "alpha",
      "--password",
      "alpha_dev_only",
      "--query",
      "CREATE DATABASE IF NOT EXISTS alpha"
    ]
  ) and
  ((.services["clickhouse-init"].ports // []) == []) and
  ((.services["clickhouse-init"].volumes // []) == [])
' "$compose_json" >/dev/null; then
  printf 'dev-stack-contract:error ClickHouse initialization is not isolated\n' >&2
  exit 1
fi

if ! jq -e '
  (
    .services.postgres.environment.POSTGRES_INITDB_ARGS ==
    "--no-locale --auth-local=scram-sha-256"
  )
' "$compose_json" >/dev/null; then
  printf 'dev-stack-contract:error PostgreSQL init policy is incomplete\n' >&2
  exit 1
fi

dev_up_dry_run="$(
  just --justfile "$repo_root/justfile" \
    --working-directory "$repo_root" \
    --dry-run dev-up 2>&1
)"

[[ "$dev_up_dry_run" == *"up -d --wait --wait-timeout 120"* ]] || {
  printf 'dev-stack-contract:error dev-up has no bounded Compose wait\n' >&2
  exit 1
}

jq -e '
  (.volumes | keys | sort) == (
    [
      "clickhouse-data",
      "minio-data",
      "nats-data",
      "postgres-data",
      "victoriametrics-data"
    ] | sort
  ) and
  all(
    .volumes | to_entries[];
    (.value.name | startswith("alpha-desk-dev_"))
  ) and
  (.networks.default.name == "alpha-desk-dev_network") and
  (.networks.default.internal != true) and
  (.networks.default.driver == "bridge") and
  (
    .networks.default.driver_opts[
      "com.docker.network.bridge.host_binding_ipv4"
    ] == "127.0.0.1"
  )
' "$compose_json" >/dev/null || {
  printf 'dev-stack-contract:error default network must be a loopback-bound bridge\n' >&2
  exit 1
}

jq -e --slurpfile lock "$lock_file" '
  [
    .services
    | to_entries[]
    | {service: .key, image: .value.image}
  ] | sort_by(.service)
  ==
  (
    $lock[0].images
    | map({service: .service, image: .compose_ref})
    | sort_by(.service)
  )
' "$compose_json" >/dev/null

clickhouse_image="$(
  jq -er '
    .images[]
    | select(.service == "clickhouse")
    | .compose_ref
  ' "$lock_file"
)"
if ! clickhouse_config_target="$(
  jq -er '
    [
      .services.clickhouse.volumes[]
      | select(.type == "bind")
      | select(
          .source
          | endswith("/infra/docker-compose/clickhouse/config.xml")
        )
    ] as $mounts
    | if (
        ($mounts | length) == 1 and
        $mounts[0].read_only == true
      )
      then $mounts[0].target
      else error("expected one read-only ClickHouse config bind")
      end
  ' "$compose_json"
)"; then
  printf 'dev-stack-contract:error invalid ClickHouse config bind\n' >&2
  exit 1
fi

[[ "$clickhouse_config_target" == \
  "/etc/clickhouse-server/config.d/zz-alpha-desk.xml" ]] || {
  printf 'dev-stack-contract:error unexpected ClickHouse config target %q\n' \
    "$clickhouse_config_target" >&2
  exit 1
}

clickhouse_config_keys=(
  background_schedule_pool_size
  listen_host
  logger.log
  logger.errorlog
  logger.level
  logger.console
)
clickhouse_config_expected=(
  128
  0.0.0.0
  ""
  ""
  information
  true
)

for index in "${!clickhouse_config_keys[@]}"; do
  key="${clickhouse_config_keys[$index]}"
  expected="${clickhouse_config_expected[$index]}"
  if ! actual="$(
    docker run \
      --rm \
      --pull=never \
      --network none \
      --read-only \
      --user 101:101 \
      --cap-drop ALL \
      --security-opt no-new-privileges:true \
      --volume \
      "$script_dir/clickhouse/config.xml:$clickhouse_config_target:ro" \
      --entrypoint /usr/bin/clickhouse \
      "$clickhouse_image" \
      extract-from-config \
      --config-file /etc/clickhouse-server/config.xml \
      --key "$key" 2>&1
  )"; then
    printf 'dev-stack-contract:error ClickHouse %s extraction failed: %s\n' \
      "$key" "$actual" >&2
    exit 1
  fi

  [[ "$actual" == "$expected" ]] || {
    printf 'dev-stack-contract:error unexpected ClickHouse %s %q\n' \
      "$key" "$actual" >&2
    exit 1
  }
done

if extra_listen_host="$(
  docker run \
    --rm \
    --pull=never \
    --network none \
    --read-only \
    --user 101:101 \
    --cap-drop ALL \
    --security-opt no-new-privileges:true \
    --volume \
    "$script_dir/clickhouse/config.xml:$clickhouse_config_target:ro" \
    --entrypoint /usr/bin/clickhouse \
    "$clickhouse_image" \
    extract-from-config \
    --config-file /etc/clickhouse-server/config.xml \
    --key 'listen_host[1]' 2>&1
)"; then
  printf 'dev-stack-contract:error unexpected additional ClickHouse listen_host %q\n' \
    "$extra_listen_host" >&2
  exit 1
fi

[[ "$extra_listen_host" == *"Not found: listen_host[1]"* ]] || {
  printf 'dev-stack-contract:error unexpected ClickHouse listener validation %s\n' \
    "$extra_listen_host" >&2
  exit 1
}

fake_bin="$tmp_dir/fake-bin"
mkdir -p "$fake_bin"

cat >"$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
url="${*: -1}"
case "$url" in
  *8222/healthz*) printf '%s\n' '{"status":"ok"}' ;;
  *8123/ping) printf 'Ok.\n' ;;
  *9000/minio/health/live) : ;;
  *13133/) : ;;
  *8428/health) : ;;
  *8428/api/v1/targets)
    printf '%s\n' '{"status":"success","data":{"activeTargets":[{"health":"up","labels":{"job":"otel-collector-internal"}},{"health":"up","labels":{"job":"alpha-desk-otlp-metrics"}},{"health":"up","labels":{"job":"victoriametrics"}}]}}'
    ;;
  *) exit 22 ;;
esac
EOF

cat >"$fake_bin/pg_isready" <<'EOF'
#!/usr/bin/env bash
printf '127.0.0.1:5432 - accepting connections\n'
EOF

cat >"$fake_bin/psql" <<'EOF'
#!/usr/bin/env bash
printf 'alpha|alpha\n'
EOF

cat >"$fake_bin/docker" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

scenario="${FAKE_DOCKER_SCENARIO:-success}"
expected_compose_file="${EXPECTED_COMPOSE_FILE:?EXPECTED_COMPOSE_FILE is required}"
nats_id="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
clickhouse_id="2223456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
minio_id="abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
extra_id="1111111111111111111111111111111111111111111111111111111111111111"

case "${1:-}" in
  compose)
    [[ "$#" -eq 9 ]] || exit 2
    [[ "$2" == "--project-name" ]] || exit 2
    [[ "$3" == "alpha-desk-dev" ]] || exit 2
    [[ "$4" == "-f" ]] || exit 2
    [[ "$5" == "$expected_compose_file" ]] || exit 2
    [[ "$expected_compose_file" == /* ]] || exit 2
    [[ "$6" == "ps" ]] || exit 2
    [[ "$7" == "--all" ]] || exit 2
    [[ "$8" == "--quiet" ]] || exit 2
    service="$9"
    case "$scenario:$service" in
      nats-missing:nats-init|clickhouse-missing:clickhouse-init|minio-missing:minio-init)
        :
        ;;
      nats-multiple:nats-init)
        printf '%s\n%s\n' "$nats_id" "$extra_id"
        ;;
      nats-unsafe:nats-init)
        printf '%s\n' 'not-a-container-id'
        ;;
      *:nats-init)
        printf '%s\n' "$nats_id"
        ;;
      *:clickhouse-init)
        printf '%s\n' "$clickhouse_id"
        ;;
      *:minio-init)
        printf '%s\n' "$minio_id"
        ;;
      *)
        exit 2
        ;;
    esac
    ;;
  inspect)
    [[ "$#" -eq 6 ]] || exit 2
    [[ "$2" == "--type" ]] || exit 2
    [[ "$3" == "container" ]] || exit 2
    [[ "$4" == "--format" ]] || exit 2
    [[ "$5" == '{{json .State}}' ]] || exit 2
    container_id="$6"
    [[ "$container_id" == "$nats_id" ||
      "$container_id" == "$clickhouse_id" ||
      "$container_id" == "$minio_id" ]] ||
      exit 2
    if [[ "$container_id" == "$nats_id" && "$scenario" == "nats-running" ]]; then
      printf '%s\n' '{"Status":"running","Running":true,"ExitCode":0}'
    elif [[ "$container_id" == "$nats_id" && "$scenario" == "nats-nonzero" ]]; then
      printf '%s\n' '{"Status":"exited","Running":false,"ExitCode":17}'
    elif [[ "$container_id" == "$clickhouse_id" && "$scenario" == "clickhouse-nonzero" ]]; then
      printf '%s\n' '{"Status":"exited","Running":false,"ExitCode":17}'
    else
      printf '%s\n' '{"Status":"exited","Running":false,"ExitCode":0}'
    fi
    ;;
  *)
    exit 2
    ;;
esac
EOF

chmod +x \
  "$fake_bin/curl" \
  "$fake_bin/docker" \
  "$fake_bin/pg_isready" \
  "$fake_bin/psql"

command_drift_failures=0

if EXPECTED_COMPOSE_FILE="$compose_file" \
  "$fake_bin/docker" compose ps nats-init >/dev/null 2>&1; then
  printf 'dev-stack-contract:error fake Docker accepted incomplete Compose argv\n' >&2
  ((command_drift_failures += 1))
fi

if EXPECTED_COMPOSE_FILE="$compose_file" \
  "$fake_bin/docker" inspect \
    0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    >/dev/null 2>&1; then
  printf 'dev-stack-contract:error fake Docker accepted incomplete inspect argv\n' >&2
  ((command_drift_failures += 1))
fi

((command_drift_failures == 0)) || exit 1

wait_output="$(
  PATH="$fake_bin:$PATH" \
    EXPECTED_COMPOSE_FILE="$compose_file" \
    DEV_STACK_WAIT_TIMEOUT_SECONDS=0 \
    DEV_STACK_WAIT_POLL_SECONDS=0 \
    "$wait_script"
)"

expected_wait_output="$(
  printf '%s\n' \
    'nats:ok' \
    'clickhouse:ok' \
    'postgres:ok' \
    'minio:ok' \
    'otel:ok' \
    'victoriametrics:ok'
)"

[[ "$wait_output" == "$expected_wait_output" ]] || {
  printf 'dev-stack-contract:error unexpected wait output\n%s\n' "$wait_output" >&2
  exit 1
}

for scenario in \
  nats-missing \
  nats-running \
  nats-nonzero \
  nats-multiple \
  nats-unsafe \
  clickhouse-missing \
  clickhouse-nonzero \
  minio-missing; do
  if PATH="$fake_bin:$PATH" \
    EXPECTED_COMPOSE_FILE="$compose_file" \
    FAKE_DOCKER_SCENARIO="$scenario" \
    DEV_STACK_WAIT_TIMEOUT_SECONDS=0 \
    DEV_STACK_WAIT_POLL_SECONDS=0 \
    "$wait_script" >"$tmp_dir/$scenario.out" 2>"$tmp_dir/$scenario.err"; then
    printf 'dev-stack-contract:error initializer scenario %s was accepted\n' \
      "$scenario" >&2
    exit 1
  fi

  if [[ "$scenario" == minio-* ]]; then
    grep -qx 'minio:error timeout' "$tmp_dir/$scenario.err"
  elif [[ "$scenario" == clickhouse-* ]]; then
    grep -qx 'clickhouse:error timeout' "$tmp_dir/$scenario.err"
  else
    grep -qx 'nats:error timeout' "$tmp_dir/$scenario.err"
  fi
done

cat >"$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
url="${*: -1}"
case "$url" in
  *8222/healthz*) printf '%s\n' '{"status":"starting"}' ;;
  *) exit 22 ;;
esac
EOF
chmod +x "$fake_bin/curl"

if PATH="$fake_bin:$PATH" \
  EXPECTED_COMPOSE_FILE="$compose_file" \
  FAKE_DOCKER_SCENARIO=success \
  DEV_STACK_WAIT_TIMEOUT_SECONDS=0 \
  DEV_STACK_WAIT_POLL_SECONDS=0 \
  "$wait_script" >"$tmp_dir/false-positive.out" 2>"$tmp_dir/false-positive.err"; then
  printf 'dev-stack-contract:error invalid NATS response was accepted\n' >&2
  exit 1
fi

grep -qx 'nats:error timeout' "$tmp_dir/false-positive.err"

printf 'dev-stack-contract:ok\n'
