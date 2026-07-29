#!/usr/bin/env bash
set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
readonly repository_root
readonly postgres_image='docker.io/library/postgres:18.4-alpine3.24@sha256:9a8afca54e7861fd90fab5fdf4c42477a6b1cb7d293595148e674e0a3181de15'
run_id="$(date -u '+%Y%m%dT%H%M%SZ')-$$"
readonly run_id
readonly evidence_root="${repository_root}/target/evidence/capture-e2e/${run_id}"
secret_root="$(mktemp -d /tmp/alpha-desk-capture-e2e.XXXXXX)"
readonly secret_root
readonly postgres_url_file="${secret_root}/postgres-url"
readonly postgres_container="alpha-desk-capture-e2e-$$"
readonly nats_container="alpha-desk-capture-e2e-nats-$$"
readonly docker_network="alpha-desk-capture-e2e-$$"
readonly config_path="${evidence_root}/capture.toml"
readonly status_path="${evidence_root}/capture-status.json"
readonly archive_path="${evidence_root}/archive"
readonly source_root="${evidence_root}/node-source"
readonly source_leaf="${source_root}/1721000000/20260728"
readonly service_stdout="${evidence_root}/service.stdout"
readonly service_stderr="${evidence_root}/service.stderr"
readonly report_path="${evidence_root}/report.json"
readonly report_staging="${evidence_root}/.report.json.tmp"
readonly block_count="${CAPTURE_E2E_BLOCKS:-3}"
readonly block_delay_millis="${CAPTURE_E2E_BLOCK_DELAY_MILLIS:-10}"
readonly minimum_runtime_seconds="${CAPTURE_E2E_MIN_RUNTIME_SECONDS:-0}"
readonly first_height="$((9001000000 + ($$ % 100000)))"
readonly last_height="$((first_height + block_count - 1))"
readonly chain_id="fixture-e2e-${run_id}"
capture_pid=''
source_writer_pid=''
process_started_at_epoch=''
max_rss_kib=0
nats_client_port=''
nats_monitor_port=''
restart_count=0

cleanup() {
  local exit_status=$?
  if [[ -n "$capture_pid" ]] && kill -0 "$capture_pid" >/dev/null 2>&1; then
    kill -TERM "$capture_pid" >/dev/null 2>&1 || true
    wait "$capture_pid" >/dev/null 2>&1 || true
  fi
  if [[ -n "$source_writer_pid" ]] && kill -0 "$source_writer_pid" >/dev/null 2>&1; then
    kill -TERM "$source_writer_pid" >/dev/null 2>&1 || true
    wait "$source_writer_pid" >/dev/null 2>&1 || true
  fi
  docker rm -f "$postgres_container" >/dev/null 2>&1 || true
  docker rm -f "$nats_container" >/dev/null 2>&1 || true
  docker network rm "$docker_network" >/dev/null 2>&1 || true
  case "$secret_root" in
    /tmp/alpha-desk-capture-e2e.*)
      if command -v trash >/dev/null 2>&1; then
        trash "$secret_root" >/dev/null 2>&1 || true
      else
        rm -rf -- "$secret_root"
      fi
      ;;
  esac
  exit "$exit_status"
}
trap cleanup EXIT INT TERM

for command_name in cargo curl docker git jq mktemp; do
  command -v "$command_name" >/dev/null 2>&1 || {
    printf 'capture-e2e:error missing command %s\n' "$command_name" >&2
    exit 2
  }
done
if ! [[ "$block_count" =~ ^[0-9]+$ ]] ||
  ! ((block_count >= 1 && block_count <= 10000000)); then
  printf '%s\n' 'capture-e2e:error CAPTURE_E2E_BLOCKS must be between 1 and 10000000' >&2
  exit 2
fi
if ! [[ "$block_delay_millis" =~ ^[0-9]+$ ]] ||
  ! ((block_delay_millis <= 60000)); then
  printf '%s\n' 'capture-e2e:error CAPTURE_E2E_BLOCK_DELAY_MILLIS must be at most 60000' >&2
  exit 2
fi
if ! [[ "$minimum_runtime_seconds" =~ ^[0-9]+$ ]] ||
  ! ((minimum_runtime_seconds <= 86400)); then
  printf '%s\n' 'capture-e2e:error CAPTURE_E2E_MIN_RUNTIME_SECONDS must be at most 86400' >&2
  exit 2
fi

mkdir -p "$evidence_root" "$archive_path" "$source_leaf"
"${repository_root}/tools/dev/ensure-nats-dev-credentials.sh" >/dev/null
set -a
# shellcheck disable=SC1091
source "${repository_root}/state/dev/nats.env"
set +a
readonly nats_password_path="${repository_root}/state/dev/nats-capture.password"
docker network create \
  --driver bridge \
  --opt com.docker.network.bridge.host_binding_ipv4=127.0.0.1 \
  "$docker_network" >/dev/null
docker run -d --name "$nats_container" \
  --network "$docker_network" \
  --network-alias nats \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges:true \
  --tmpfs /data:rw,noexec,nosuid,nodev,size=512m,mode=0700 \
  --tmpfs /tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777 \
  -e NATS_BOOTSTRAP_USER="$ALPHA_DESK_NATS_BOOTSTRAP_USER" \
  -e NATS_BOOTSTRAP_PASSWORD="$ALPHA_DESK_NATS_BOOTSTRAP_PASSWORD" \
  -e NATS_CAPTURE_USER="$ALPHA_DESK_NATS_CAPTURE_USER" \
  -e NATS_CAPTURE_PASSWORD="$ALPHA_DESK_NATS_CAPTURE_PASSWORD" \
  -e NATS_READER_USER="$ALPHA_DESK_NATS_READER_USER" \
  -e NATS_READER_PASSWORD="$ALPHA_DESK_NATS_READER_PASSWORD" \
  -p 127.0.0.1::4222 \
  -p 127.0.0.1::8222 \
  -v "${repository_root}/infra/docker-compose/nats/nats.conf:/etc/nats/nats.conf:ro" \
  docker.io/library/nats:2.14.3-alpine@sha256:c11af972c99ae542de8925e6a7d9c533aa1eb039660420d2074beed6089b3bf0 \
  nats-server --config /etc/nats/nats.conf >/dev/null
nats_client_port="$(docker port "$nats_container" 4222/tcp | sed -E 's/.*:([0-9]+)$/\1/')"
nats_monitor_port="$(docker port "$nats_container" 8222/tcp | sed -E 's/.*:([0-9]+)$/\1/')"
[[ "$nats_client_port" =~ ^[0-9]+$ && "$nats_monitor_port" =~ ^[0-9]+$ ]] || {
  printf '%s\n' 'capture-e2e:error invalid NATS published ports' >&2
  exit 1
}
for attempt in $(seq 1 80); do
  if curl --fail --silent \
    "http://127.0.0.1:${nats_monitor_port}/healthz?js-enabled-only=true" \
    | jq -e '.status == "ok"' >/dev/null 2>&1; then
    break
  fi
  if [[ "$attempt" == 80 ]]; then
    docker logs "$nats_container" >"${evidence_root}/nats.log" 2>&1 || true
    printf '%s\n' 'capture-e2e:error NATS readiness timeout' >&2
    exit 1
  fi
  sleep 0.25
done
docker run --rm \
  --network "$docker_network" \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges:true \
  --tmpfs /tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777 \
  -e NATS_URL=nats://nats:4222 \
  -e NATS_BOOTSTRAP_POLICY=/opt/alpha-desk/bootstrap.json \
  -e NATS_BOOTSTRAP_USER="$ALPHA_DESK_NATS_BOOTSTRAP_USER" \
  -e NATS_BOOTSTRAP_PASSWORD="$ALPHA_DESK_NATS_BOOTSTRAP_PASSWORD" \
  -e XDG_CONFIG_HOME=/tmp/config \
  -e XDG_DATA_HOME=/tmp/data \
  -v "${repository_root}/infra/docker-compose/nats/init-streams.sh:/opt/alpha-desk/init-streams.sh:ro" \
  -v "${repository_root}/infra/docker-compose/nats/bootstrap.json:/opt/alpha-desk/bootstrap.json:ro" \
  --entrypoint /bin/sh \
  docker.io/natsio/nats-box:0.19.7-nonroot@sha256:e86b9681f330ab1aa45744dd5cb367d44205b28fac2519a2f74ca0255803161a \
  /opt/alpha-desk/init-streams.sh >/dev/null

docker run -d --name "$postgres_container" \
  --read-only \
  --user 70:70 \
  --cap-drop ALL \
  --security-opt no-new-privileges:true \
  --tmpfs /var/lib/postgresql:rw,noexec,nosuid,nodev,uid=70,gid=70,mode=0700 \
  --tmpfs /var/run/postgresql:rw,noexec,nosuid,nodev,uid=70,gid=70,mode=0770 \
  --tmpfs /tmp:rw,noexec,nosuid,nodev,uid=70,gid=70,mode=1777 \
  -e POSTGRES_DB=alpha \
  -e POSTGRES_USER=alpha \
  -e POSTGRES_PASSWORD=alpha_dev_only \
  -p 127.0.0.1::5432 \
  -v "${repository_root}/infra/docker-compose/postgres/init.sql:/docker-entrypoint-initdb.d/0000-init.sql:ro" \
  -v "${repository_root}/schemas/postgres/0001_capture_incidents.sql:/docker-entrypoint-initdb.d/0001-capture-incidents.sql:ro" \
  -v "${repository_root}/schemas/postgres/0002_capture_progress.sql:/docker-entrypoint-initdb.d/0002-capture-progress.sql:ro" \
  "$postgres_image" >/dev/null

for attempt in $(seq 1 80); do
  if docker exec "$postgres_container" pg_isready -U alpha -d alpha >/dev/null 2>&1; then
    break
  fi
  if [[ "$attempt" == 80 ]]; then
    docker logs "$postgres_container" >"${evidence_root}/postgres.log" 2>&1 || true
    printf '%s\n' 'capture-e2e:error PostgreSQL readiness timeout' >&2
    exit 1
  fi
  sleep 0.25
done

postgres_port="$(docker port "$postgres_container" 5432/tcp | sed -E 's/.*:([0-9]+)$/\1/')"
[[ "$postgres_port" =~ ^[0-9]+$ ]] || {
  printf '%s\n' 'capture-e2e:error invalid PostgreSQL published port' >&2
  exit 1
}
umask 077
printf 'postgresql://alpha:alpha_dev_only@127.0.0.1:%s/alpha?sslmode=disable' \
  "$postgres_port" >"$postgres_url_file"
chmod 600 "$postgres_url_file"

cat >"$config_path" <<EOF
parser_version = "synthetic-fixture-parser-v1"

[runtime]
chain_id = "${chain_id}"
first_height = ${first_height}
archive_path = "${archive_path}"
status_path = "${status_path}"
postgres_url_path = "${postgres_url_file}"
nats_server_url = "nats://127.0.0.1:${nats_client_port}"
nats_stream = "HL_CANONICAL"
nats_username = "${ALPHA_DESK_NATS_CAPTURE_USER}"
nats_password_path = "${nats_password_path}"
max_pending_blocks = 4096
retained_committed_blocks = 4096
publisher_ledger_capacity = 100000
nats_max_ack_inflight = 4096
publish_timeout_millis = 5000
backpressure_timeout_millis = 5000
shutdown_grace_millis = 15000
disk_reserve_bytes = 1073741824

[spool]
path = "${evidence_root}/spool"
segment_target_bytes = 67108864
rotation_interval_seconds = 300

[spool.committed_durability]
mode = "fsync-every-record"

[spool.provisional_durability]
mode = "batched"
max_records = 128
max_delay_millis = 100

[[sources]]
id = "synthetic-fixture"
source_version = "synthetic-node-v1"
trust = "locally-verified-committed"
class = "committed-block"
queue_capacity = 4096
max_payload_bytes = 8388608
adapter = { kind = "node-block-directory", path = "${source_root}", stream_name = "synthetic-fixture", start_height = ${first_height}, poll_interval_millis = 25 }
EOF
chmod 600 "$config_path"

cargo +1.97.1 build -p hl-capture -p archive-inspect -p spool-inspect --locked --offline
: >"$service_stdout"
: >"$service_stderr"

write_source_block() {
  local height="$1"
  local parent_height="$((height - 1))"
  local staging="${source_leaf}/.${height}.tmp"
  jq -n \
    --argjson height "$height" \
    --argjson parent_height "$parent_height" \
    '{
      abci_block: {
        time: "2026-07-28T12:00:00.000000000",
        round: $height,
        parent_round: $parent_height,
        proposer: "0x5ac99df645f3414876c816caa18b2d234024b487"
      },
      signed_action_bundles: []
    }' >"$staging"
  mv "$staging" "${source_leaf}/${height}"
}

start_capture() {
  "${repository_root}/target/debug/hl-capture" \
    run \
    --config "$config_path" \
    >>"$service_stdout" 2>>"$service_stderr" &
  capture_pid=$!
  if [[ -z "$process_started_at_epoch" ]]; then
    process_started_at_epoch="$(date -u '+%s')"
  fi
}

start_source_writer() {
  local start_height="$1"
  local end_height="$2"
  (
    local height="$start_height"
    while ((height <= end_height)); do
      sleep "$(awk -v millis="$block_delay_millis" 'BEGIN { printf "%.3f", millis / 1000 }')"
      write_source_block "$height"
      height="$((height + 1))"
    done
  ) &
  source_writer_pid=$!
}

sample_process() {
  local rss_kib
  rss_kib="$(ps -o rss= -p "$capture_pid" 2>/dev/null | awk '{print $1}' || true)"
  if [[ "$rss_kib" =~ ^[0-9]+$ ]] && ((rss_kib > max_rss_kib)); then
    max_rss_kib="$rss_kib"
  fi
}

expected_processing_seconds="$(((block_count * block_delay_millis + 999) / 1000))"
readiness_timeout_seconds="$minimum_runtime_seconds"
if ((expected_processing_seconds > readiness_timeout_seconds)); then
  readiness_timeout_seconds="$expected_processing_seconds"
fi
readiness_attempts="$(((readiness_timeout_seconds + 120) * 4))"

wait_for_durable_height() {
  local expected_height="$1"
  local attempt
  for attempt in $(seq 1 "$readiness_attempts"); do
    if ! kill -0 "$capture_pid" >/dev/null 2>&1; then
      wait "$capture_pid" || true
      capture_pid=''
      printf '%s\n' 'capture-e2e:error capture process exited before reaching durable height' >&2
      exit 1
    fi
    sample_process
    if [[ -f "$status_path" ]] \
      && jq -e \
        --argjson expected "$expected_height" \
        '.ready == true and .health == "green" and .durable_height == $expected and .pending_blocks == 0' \
        "$status_path" >/dev/null 2>&1; then
      return
    fi
    sleep 0.25
  done
  printf '%s\n' 'capture-e2e:error durable-height timeout' >&2
  exit 1
}

stop_capture() {
  local expected_height="$1"
  kill -TERM "$capture_pid"
  if ! wait "$capture_pid"; then
    capture_pid=''
    printf '%s\n' 'capture-e2e:error capture process did not shut down cleanly' >&2
    exit 1
  fi
  capture_pid=''
  jq -e \
    --argjson expected "$expected_height" \
    '.ready == false and .health == "yellow" and .durable_height == $expected and .pending_blocks == 0' \
    "$status_path" >/dev/null
}

write_source_block "$first_height"
if ((block_count >= 2)); then
  start_capture
  wait_for_durable_height "$first_height"
  stop_capture "$first_height"
  restart_count=1
  write_source_block "$((first_height + 1))"
  start_capture
  if ((block_count >= 3)); then
    start_source_writer "$((first_height + 2))" "$last_height"
  fi
else
  start_capture
fi
wait_for_durable_height "$last_height"
if [[ -n "$source_writer_pid" ]]; then
  wait "$source_writer_pid"
  source_writer_pid=''
fi

while (( $(date -u '+%s') - process_started_at_epoch < minimum_runtime_seconds )); do
  if ! kill -0 "$capture_pid" >/dev/null 2>&1; then
    wait "$capture_pid" || true
    capture_pid=''
    printf '%s\n' 'capture-e2e:error capture process exited during minimum runtime' >&2
    exit 1
  fi
  jq -e \
    --argjson expected "$last_height" \
    '.ready == true and .health == "green" and .durable_height == $expected and .pending_blocks == 0' \
    "$status_path" >/dev/null
  sample_process
  sleep 1
done

stop_capture "$last_height"

archive_summary="$("${repository_root}/target/debug/archive-inspect" verify "$archive_path")"
[[ "$archive_summary" == *"blocks=${block_count}"* ]] || {
  printf '%s\n' 'capture-e2e:error archive block count mismatch' >&2
  exit 1
}
spool_summary="$("${repository_root}/target/debug/spool-inspect" \
  verify "${evidence_root}/spool/synthetic-fixture")"
[[ "$spool_summary" == *"records=${block_count}"* ]] || {
  printf '%s\n' 'capture-e2e:error spool record count mismatch' >&2
  exit 1
}
archived_blocks="$(docker exec "$postgres_container" \
  psql -U alpha -d alpha -Atqc \
  "SELECT count(*) FROM capture_archived_blocks WHERE chain_id = '${chain_id}' AND state = 'acknowledged'")"
acknowledged_publications="$(docker exec "$postgres_container" \
  psql -U alpha -d alpha -Atqc \
  "SELECT count(*) FROM capture_block_publications WHERE chain_id = '${chain_id}' AND ack_stream_sequence IS NOT NULL")"
durable_height="$(docker exec "$postgres_container" \
  psql -U alpha -d alpha -Atqc \
  "SELECT committed_block_height::text FROM capture_sequencer_cursors WHERE chain_id = '${chain_id}'")"
[[ "$archived_blocks" == "$block_count" ]] || {
  printf '%s\n' 'capture-e2e:error acknowledged archive count mismatch' >&2
  exit 1
}
[[ "$acknowledged_publications" == "$block_count" ]] || {
  printf '%s\n' 'capture-e2e:error publication count mismatch' >&2
  exit 1
}
[[ "$durable_height" == "$last_height" ]] || {
  printf '%s\n' 'capture-e2e:error durable cursor mismatch' >&2
  exit 1
}

binary_sha256="$(shasum -a 256 "${repository_root}/target/debug/hl-capture" | awk '{print $1}')"
postgres_version="$(docker exec "$postgres_container" postgres --version)"
nats_version="$(curl --fail --silent --show-error "http://127.0.0.1:${nats_monitor_port}/varz" | jq -r '.version')"
elapsed_seconds="$(( $(date -u '+%s') - process_started_at_epoch ))"
archive_bytes="$(( $(du -sk "$archive_path" | awk '{print $1}') * 1024 ))"
service_stdout_bytes="$(wc -c <"$service_stdout" | tr -d '[:space:]')"
service_stderr_bytes="$(wc -c <"$service_stderr" | tr -d '[:space:]')"
jq -n \
  --arg schema_version 'hl.capture.e2e.v1' \
  --arg run_id "$run_id" \
  --arg chain_id "$chain_id" \
  --argjson first_height "$first_height" \
  --argjson last_height "$last_height" \
  --argjson block_count "$block_count" \
  --argjson acknowledged_publications "$acknowledged_publications" \
  --argjson restart_count "$restart_count" \
  --argjson minimum_runtime_seconds "$minimum_runtime_seconds" \
  --argjson elapsed_seconds "$elapsed_seconds" \
  --argjson max_rss_kib "$max_rss_kib" \
  --argjson archive_bytes "$archive_bytes" \
  --argjson service_stdout_bytes "$service_stdout_bytes" \
  --argjson service_stderr_bytes "$service_stderr_bytes" \
  --arg archive_summary "$archive_summary" \
  --arg spool_summary "$spool_summary" \
  --arg binary_sha256 "$binary_sha256" \
  --arg postgres_version "$postgres_version" \
  --arg nats_version "$nats_version" \
  '{
    schema_version: $schema_version,
    run_id: $run_id,
    mode: "synthetic-node-source",
    live_source_qualified: false,
    chain_id: $chain_id,
    first_height: $first_height,
    last_height: $last_height,
    block_count: $block_count,
    acknowledged_publications: $acknowledged_publications,
    restart_count: $restart_count,
    minimum_runtime_seconds: $minimum_runtime_seconds,
    elapsed_seconds: $elapsed_seconds,
    max_rss_kib: $max_rss_kib,
    archive_bytes: $archive_bytes,
    service_stdout_bytes: $service_stdout_bytes,
    service_stderr_bytes: $service_stderr_bytes,
    archive_summary: $archive_summary,
    spool_summary: $spool_summary,
    binary_sha256: $binary_sha256,
    postgres_version: $postgres_version,
    nats_version: $nats_version,
    clean_shutdown: true
  }' >"$report_staging"
mv "$report_staging" "$report_path"

printf 'capture-e2e:ok report=%s blocks=%s publications=%s\n' \
  "$report_path" "$block_count" "$acknowledged_publications"
