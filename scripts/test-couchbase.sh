#!/usr/bin/env bash
# Starts a throwaway Couchbase Server (Community) for the driver tests.
#
#   scripts/test-couchbase.sh up     start a one-node cluster, load sample documents and print the variables to export
#   scripts/test-couchbase.sh down   remove the container
#
# Load the variables with:  eval "$(scripts/test-couchbase.sh up)"
# Then:  cargo test -p dbclient-driver-couchbase
set -euo pipefail

NAME=dbclient-test-couchbase
# Pinned so a new server release cannot change what the tests see unnoticed.
IMAGE="${DBCLIENT_TEST_COUCHBASE_IMAGE:-couchbase:community-8.0.2}"
USER=Administrator
# For this container only.
PASSWORD=Test-pass1

down() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

cli() { docker exec "$NAME" couchbase-cli "$@" -c 127.0.0.1 -u "$USER" -p "$PASSWORD"; }

sql() {
  local url="$1" statement="$2"
  curl -fsS -u "$USER:$PASSWORD" "$url/query/service" -H 'Content-Type: application/json' \
    -d "$(printf '{"statement": %s}' "$(printf '%s' "$statement" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')")"
}

up() {
  down
  # Nothing is mounted: the data disappears with the container.
  docker run -d --name "$NAME" --memory=2g -p 127.0.0.1::8093 "$IMAGE" >/dev/null
  for _ in $(seq 1 90); do
    if docker exec "$NAME" curl -fsS -o /dev/null http://127.0.0.1:8091/ui/index.html 2>/dev/null; then break; fi
    sleep 1
  done
  docker exec "$NAME" couchbase-cli cluster-init -c 127.0.0.1 \
    --cluster-username "$USER" --cluster-password "$PASSWORD" \
    --services data,index,query --cluster-ramsize 512 --cluster-index-ramsize 256 \
    --index-storage-setting default >/dev/null
  # Community edition has no Magma storage, and one node cannot hold replicas.
  cli bucket-create --bucket shop --bucket-type couchbase --bucket-ramsize 128 \
    --bucket-replica 0 --storage-backend couchstore --wait >/dev/null

  local url
  url="http://127.0.0.1:$(docker port "$NAME" 8093 | head -1 | sed 's/.*://')"
  # The query service learns about a new bucket a moment later.
  for _ in $(seq 1 60); do
    if sql "$url" "CREATE SCOPE shop.sales IF NOT EXISTS" 2>/dev/null | grep -q '"status": "success"'; then break; fi
    sleep 1
  done
  sql "$url" "CREATE COLLECTION shop.sales.orders IF NOT EXISTS" >/dev/null
  # Empty, for the tests that write.
  sql "$url" "CREATE COLLECTION shop.sales.drafts IF NOT EXISTS" >/dev/null
  # 25 orders: odd numbers in İzmir, even ones in Ankara; every fifth has no note.
  local values="" i city note
  for i in $(seq 1 25); do
    city=Ankara; [ $((i % 2)) -eq 1 ] && city=İzmir
    note=""; [ $((i % 5)) -ne 0 ] && note=", \"note\": \"Çağrı $i\""
    values+="(\"o$(printf %02d "$i")\", {\"city\": \"$city\", \"total\": $i.5, \"items\": [$i, $((i + 1))]$note}),"
  done
  for _ in $(seq 1 30); do
    if sql "$url" "INSERT INTO shop.sales.orders (KEY, VALUE) VALUES ${values%,}" 2>/dev/null | grep -q '"status": "success"'; then break; fi
    sleep 1
  done
  sql "$url" "INSERT INTO shop._default._default (KEY, VALUE) VALUES (\"settings\", {\"theme\": \"dark\"})" >/dev/null

  echo "export DBCLIENT_TEST_COUCHBASE_URL=$url"
  echo "export DBCLIENT_TEST_COUCHBASE_USER=$USER"
  echo "export DBCLIENT_TEST_COUCHBASE_PASSWORD=$PASSWORD"
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
