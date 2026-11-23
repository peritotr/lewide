#!/usr/bin/env bash
# Starts a throwaway Redis server with a password for the driver tests.
#
#   scripts/test-redis.sh up     start it and print the variable the tests read
#   scripts/test-redis.sh down   remove the container
set -euo pipefail

NAME=dbclient-test-redis

case "${1:-}" in
  up)
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --name "$NAME" -p 127.0.0.1::6379 redis:7 redis-server --requirepass test >/dev/null
    for _ in $(seq 1 30); do
      docker exec "$NAME" redis-cli -a test --no-auth-warning ping 2>/dev/null | grep -q PONG && break
      sleep 1
    done
    PORT="$(docker port "$NAME" 6379/tcp | head -1 | sed 's/.*://')"
    echo "export DBCLIENT_TEST_REDIS_URL='redis://:test@127.0.0.1:$PORT/0'"
    ;;
  down) docker rm -f "$NAME" >/dev/null 2>&1 || true ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
