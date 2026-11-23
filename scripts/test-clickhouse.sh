#!/usr/bin/env bash
# Starts a throwaway ClickHouse server with a password for the driver tests.
#
#   scripts/test-clickhouse.sh up     start it and print the variable the tests read
#   scripts/test-clickhouse.sh down   remove the container
set -euo pipefail

NAME=dbclient-test-clickhouse
IMAGE="${CLICKHOUSE_IMAGE:-clickhouse/clickhouse-server:25.8}"
# Only for this container.
PASSWORD=test

case "${1:-}" in
  up)
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --name "$NAME" --ulimit nofile=262144:262144 \
      -e CLICKHOUSE_USER=lewide -e CLICKHOUSE_PASSWORD="$PASSWORD" -e CLICKHOUSE_DB=dbclient_test \
      -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
      -p 127.0.0.1::8123 "$IMAGE" >/dev/null
    ready=""
    for _ in $(seq 1 60); do
      if docker exec "$NAME" clickhouse-client --user lewide --password "$PASSWORD" -q "SELECT 1" >/dev/null 2>&1; then
        ready=1
        break
      fi
      sleep 1
    done
    if [ -z "$ready" ]; then
      echo "ClickHouse did not start; see: docker logs $NAME" >&2
      exit 1
    fi
    PORT="$(docker port "$NAME" 8123/tcp | head -1 | sed 's/.*://')"
    echo "export DBCLIENT_TEST_CLICKHOUSE_URL='http://lewide:$PASSWORD@127.0.0.1:$PORT/dbclient_test'"
    ;;
  down) docker rm -f "$NAME" >/dev/null 2>&1 || true ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
