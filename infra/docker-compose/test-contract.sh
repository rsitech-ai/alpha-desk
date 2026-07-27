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

for command_name in docker jq; do
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
  (.images | length == 8) and
  all(
    .images[];
    . as $image |
    ($image.service | type == "string" and length > 0) and
    ($image.version | type == "string" and length > 0) and
    ($image.index_digest | test("^sha256:[0-9a-f]{64}$")) and
    ($image.linux_arm64_digest | test("^sha256:[0-9a-f]{64}$")) and
    ($image.compose_ref | endswith("@" + $image.index_digest))
  ) and
  (([.images[].service] | unique | length) == 8)
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

jq -e '
  (.services["nats-init"].restart == "no") and
  (.services["minio-init"].restart == "no") and
  (.services["nats-init"].depends_on.nats.condition == "service_healthy") and
  (.services["minio-init"].depends_on.minio.condition == "service_healthy") and
  (.services.victoriametrics.depends_on["otel-collector"].condition == "service_healthy")
' "$compose_json" >/dev/null

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
  (.networks.default.internal == true)
' "$compose_json" >/dev/null

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

chmod +x "$fake_bin/curl" "$fake_bin/pg_isready" "$fake_bin/psql"

wait_output="$(
  PATH="$fake_bin:$PATH" \
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
  DEV_STACK_WAIT_TIMEOUT_SECONDS=0 \
  DEV_STACK_WAIT_POLL_SECONDS=0 \
  "$wait_script" >"$tmp_dir/false-positive.out" 2>"$tmp_dir/false-positive.err"; then
  printf 'dev-stack-contract:error invalid NATS response was accepted\n' >&2
  exit 1
fi

grep -qx 'nats:error timeout' "$tmp_dir/false-positive.err"

printf 'dev-stack-contract:ok\n'
