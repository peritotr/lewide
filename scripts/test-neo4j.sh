#!/usr/bin/env bash
# Starts a throwaway Neo4j server for the Neo4j driver tests.
#
#   scripts/test-neo4j.sh up     start the container and print the export line
#   scripts/test-neo4j.sh down   remove the container
#
# The image can be changed to test another server version:
#   NEO4J_IMAGE=neo4j:5-community scripts/test-neo4j.sh up
set -euo pipefail

NAME=dbclient-test-neo4j
IMAGE="${NEO4J_IMAGE:-neo4j:community}"
# Only for this container. Neo4j refuses passwords shorter than 8 characters.
PASSWORD=testtest1

down() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

port() { docker port "$NAME" "$1" | head -1 | sed 's/.*://'; }

up() {
  down
  # Random localhost ports, so a Neo4j already running on 7474 is left alone.
  docker run -d --name "$NAME" \
    -e NEO4J_AUTH="neo4j/$PASSWORD" \
    -p 127.0.0.1::7474 -p 127.0.0.1::7687 \
    "$IMAGE" >/dev/null

  local http bolt
  http="$(port 7474)"
  bolt="$(port 7687)"

  # The HTTP port opens before the database accepts statements, so wait for
  # a statement to succeed rather than for the port. Starting takes 10-40 s.
  for _ in $(seq 1 120); do
    if curl -sf -u "neo4j:$PASSWORD" -H 'Content-Type: application/json' \
      -d '{"statement":"RETURN 1"}' \
      "http://127.0.0.1:$http/db/neo4j/query/v2" 2>/dev/null | grep -q '"values"'; then
      echo "# Neo4j is ready (HTTP $http, Bolt $bolt)"
      echo "export DBCLIENT_TEST_NEO4J_URL=http://neo4j:$PASSWORD@127.0.0.1:$http"
      return 0
    fi
    sleep 1
  done
  echo "timed out waiting for $NAME" >&2
  docker logs "$NAME" 2>&1 | tail -20 >&2
  return 1
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
