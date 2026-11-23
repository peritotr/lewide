#!/usr/bin/env bash
# Starts a throwaway Milvus server (standalone, embedded etcd, local storage) for the driver tests.
#
#   scripts/test-milvus.sh up     start the server, load sample data and print the variables to export
#   scripts/test-milvus.sh down   remove the container
#
# Load the variables with:  eval "$(scripts/test-milvus.sh up)"
# Then:  cargo test -p dbclient-driver-milvus
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/test-milvus"
NAME=dbclient-test-milvus
# Pinned so a new server release cannot change what the tests see unnoticed.
IMAGE="${DBCLIENT_TEST_MILVUS_IMAGE:-milvusdb/milvus:v3.0.6}"
# Milvus's documented default root password, for this container only. With
# authorization on, the tests can also check that a wrong one is refused.
TOKEN=root:Milvus

down() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

port() { docker port "$NAME" 19530 | head -1 | sed 's/.*://'; }

api() {
  local url="$1" path="$2" body="$3"
  curl -fsS -X POST "$url/v2/vectordb/$path" \
    -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d "$body"
}

wait_for() {
  local url="$1"
  for _ in $(seq 1 120); do
    if api "$url" collections/list '{}' 2>/dev/null | grep -q '"code":0'; then return 0; fi
    sleep 1
  done
  echo "timed out waiting for $NAME" >&2
  docker logs "$NAME" 2>&1 | tail -30 >&2
  return 1
}

# Two collections: `articles` with an integer key, one cosine vector and a
# JSON field; `products` with a text key and two named vectors (L2 and IP).
seed() {
  local url="$1"
  api "$url" collections/create '{
    "collectionName": "articles",
    "schema": {"autoId": false, "enableDynamicField": true, "fields": [
      {"fieldName": "id", "dataType": "Int64", "isPrimary": true},
      {"fieldName": "embedding", "dataType": "FloatVector", "elementTypeParams": {"dim": "4"}},
      {"fieldName": "title", "dataType": "VarChar", "elementTypeParams": {"max_length": "200"}},
      {"fieldName": "year", "dataType": "Int64"},
      {"fieldName": "tags", "dataType": "Array", "elementDataType": "VarChar", "elementTypeParams": {"max_capacity": "8", "max_length": "40"}}
    ]},
    "indexParams": [{"fieldName": "embedding", "indexName": "embedding", "metricType": "COSINE", "indexType": "AUTOINDEX"}]
  }' >/dev/null
  local rows="" i
  for i in $(seq 1 40); do
    local city=İzmir; [ $((i % 2)) -eq 0 ] && city=Ankara
    rows+="{\"id\": $i, \"embedding\": [$i, $((i % 5)).5, 1, $((40 - i))], \"title\": \"Makale $i — Çağrı\", \"year\": $((2000 + i % 25)), \"tags\": [\"t$((i % 3))\", \"all\"], \"city\": \"$city\"},"
  done
  api "$url" entities/insert "{\"collectionName\": \"articles\", \"data\": [${rows%,}]}" >/dev/null

  api "$url" collections/create '{
    "collectionName": "products",
    "schema": {"autoId": false, "enableDynamicField": false, "fields": [
      {"fieldName": "sku", "dataType": "VarChar", "isPrimary": true, "elementTypeParams": {"max_length": "32"}},
      {"fieldName": "image", "dataType": "FloatVector", "elementTypeParams": {"dim": "3"}},
      {"fieldName": "text", "dataType": "FloatVector", "elementTypeParams": {"dim": "2"}},
      {"fieldName": "price", "dataType": "Double"}
    ]},
    "indexParams": [
      {"fieldName": "image", "indexName": "image", "metricType": "L2", "indexType": "AUTOINDEX"},
      {"fieldName": "text", "indexName": "text", "metricType": "IP", "indexType": "AUTOINDEX"}
    ]
  }' >/dev/null
  api "$url" entities/insert '{"collectionName": "products", "data": [
    {"sku": "007", "image": [0, 0, 1], "text": [1, 0], "price": 9.5},
    {"sku": "a-1", "image": [1, 0, 0], "text": [0, 1], "price": 20},
    {"sku": "b-2", "image": [0, 1, 0], "text": [0.5, 0.5], "price": 35.25}
  ]}' >/dev/null
  # Loaded collections are searchable at once; creating with an index loads them.
  for c in articles products; do
    api "$url" collections/load "{\"collectionName\": \"$c\"}" >/dev/null
    api "$url" collections/flush "{\"collectionName\": \"$c\"}" >/dev/null 2>&1 || true
  done
}

up() {
  down
  mkdir -p "$OUT"
  # The settings Milvus's own standalone script uses for an embedded etcd.
  cat > "$OUT/embedEtcd.yaml" <<'YAML'
listen-client-urls: http://0.0.0.0:2379
advertise-client-urls: http://0.0.0.0:2379
quota-backend-bytes: 4294967296
auto-compaction-mode: revision
auto-compaction-retention: '1000'
YAML
  cat > "$OUT/user.yaml" <<'YAML'
common:
  security:
    authorizationEnabled: true
YAML
  # Nothing is mounted: the configuration is copied in and the data disappears with the container.
  docker create --name "$NAME" --security-opt seccomp:unconfined --memory=2g \
    -p 127.0.0.1::19530 \
    -e ETCD_USE_EMBED=true -e ETCD_DATA_DIR=/var/lib/milvus/etcd \
    -e ETCD_CONFIG_PATH=/milvus/configs/embedEtcd.yaml -e COMMON_STORAGETYPE=local \
    -e DEPLOY_MODE=STANDALONE \
    "$IMAGE" milvus run standalone >/dev/null
  docker cp "$OUT/embedEtcd.yaml" "$NAME:/milvus/configs/embedEtcd.yaml" >/dev/null
  docker cp "$OUT/user.yaml" "$NAME:/milvus/configs/user.yaml" >/dev/null
  docker start "$NAME" >/dev/null
  local url
  url="http://127.0.0.1:$(port)"
  wait_for "$url"
  seed "$url"
  echo "export DBCLIENT_TEST_MILVUS_URL=$url"
  echo "export DBCLIENT_TEST_MILVUS_TOKEN=$TOKEN"
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
