#!/usr/bin/env bash
# Starts a throwaway PostgreSQL server with the pgvector extension for the driver tests.
#
#   scripts/test-pgvector.sh up     start it and print the variable the tests read
#   scripts/test-pgvector.sh down   remove the container
#
# Load the variable with:  eval "$(scripts/test-pgvector.sh up)"
# Then:  cargo test -p dbclient-driver-pgvector
set -euo pipefail

NAME=dbclient-test-pgvector
# Pinned so a new release cannot change what the tests see unnoticed.
IMAGE="${DBCLIENT_TEST_PGVECTOR_IMAGE:-pgvector/pgvector:0.8.1-pg17}"

case "${1:-}" in
  up)
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --name "$NAME" -p 127.0.0.1::5432 -e POSTGRES_PASSWORD=test "$IMAGE" >/dev/null
    for _ in $(seq 1 60); do
      # pg_isready answers during the first start's temporary server too; a query
      # through TCP only works once the real one listens.
      docker exec "$NAME" psql -h 127.0.0.1 -U postgres -c "SELECT 1" >/dev/null 2>&1 && break
      sleep 1
    done
    docker exec "$NAME" psql -h 127.0.0.1 -U postgres -q -c "CREATE EXTENSION IF NOT EXISTS vector" >/dev/null
    PORT="$(docker port "$NAME" 5432/tcp | head -1 | sed 's/.*://')"
    echo "export DBCLIENT_TEST_PGVECTOR_URL='postgres://postgres:test@127.0.0.1:$PORT/postgres'"
    ;;
  down) docker rm -f "$NAME" >/dev/null 2>&1 || true ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
