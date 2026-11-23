#!/usr/bin/env bash
# Starts throwaway Elasticsearch and OpenSearch servers for the driver tests.
#
#   scripts/test-elasticsearch.sh up     start Elasticsearch (with a password) and print its variables
#   scripts/test-elasticsearch.sh open   also start OpenSearch (no security plugin) and print its variable
#   scripts/test-elasticsearch.sh down   remove both containers
#
# Load the variables with:  eval "$(scripts/test-elasticsearch.sh up)"
# Then:  cargo test -p dbclient-driver-elasticsearch
set -euo pipefail

NAME=dbclient-test-elasticsearch
NAME_OS=dbclient-test-opensearch
# A full disk on the developer's machine must not make the test server read-only,
# so the disk watermarks are off (the containers hold a few kilobytes).
# Pinned so a new release cannot change what the tests see unnoticed.
IMAGE="${DBCLIENT_TEST_ELASTICSEARCH_IMAGE:-docker.elastic.co/elasticsearch/elasticsearch:8.19.7}"
IMAGE_OS="${DBCLIENT_TEST_OPENSEARCH_IMAGE:-opensearchproject/opensearch:2.19.3}"
# A made-up password for this container only.
PASSWORD=testtest1

port() { docker port "$1" 9200/tcp | head -1 | sed 's/.*://'; }

wait_for() {
  local name="$1" url="$2"; shift 2
  for _ in $(seq 1 120); do
    if curl -fsS -o /dev/null "$@" "$url/_cluster/health?wait_for_status=yellow&timeout=1s" 2>/dev/null; then return 0; fi
    sleep 1
  done
  echo "timed out waiting for $name" >&2
  docker logs "$name" 2>&1 | tail -20 >&2
  return 1
}

up() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  # Security on, so the tests can check passwords; TLS off on HTTP to keep the
  # container simple (the TLS code is shared with the other HTTP drivers).
  docker run -d --name "$NAME" -p 127.0.0.1::9200 \
    -e discovery.type=single-node \
    -e xpack.security.enabled=true \
    -e xpack.security.http.ssl.enabled=false \
    -e ELASTIC_PASSWORD="$PASSWORD" \
    -e ES_JAVA_OPTS="-Xms512m -Xmx512m" \
    -e cluster.routing.allocation.disk.threshold_enabled=false \
    "$IMAGE" >/dev/null
  local url
  url="http://127.0.0.1:$(port "$NAME")"
  wait_for "$NAME" "$url" -u "elastic:$PASSWORD"
  echo "export DBCLIENT_TEST_ELASTICSEARCH_URL=$url"
  echo "export DBCLIENT_TEST_ELASTICSEARCH_USER=elastic"
  echo "export DBCLIENT_TEST_ELASTICSEARCH_PASSWORD=$PASSWORD"
}

open() {
  docker rm -f "$NAME_OS" >/dev/null 2>&1 || true
  docker run -d --name "$NAME_OS" -p 127.0.0.1::9200 \
    -e discovery.type=single-node \
    -e DISABLE_SECURITY_PLUGIN=true \
    -e DISABLE_INSTALL_DEMO_CONFIG=true \
    -e OPENSEARCH_JAVA_OPTS="-Xms512m -Xmx512m" \
    -e cluster.routing.allocation.disk.threshold_enabled=false \
    "$IMAGE_OS" >/dev/null
  local url
  url="http://127.0.0.1:$(port "$NAME_OS")"
  wait_for "$NAME_OS" "$url"
  echo "export DBCLIENT_TEST_OPENSEARCH_URL=$url"
}

case "${1:-}" in
  up) up ;;
  open) open ;;
  down) docker rm -f "$NAME" "$NAME_OS" >/dev/null 2>&1 || true ;;
  *) echo "usage: $0 up|open|down" >&2; exit 2 ;;
esac
