#!/usr/bin/env bash
# Starts a throwaway InfluxDB 3 Core server (in-memory storage) for the driver tests.
#
#   scripts/test-influxdb.sh up     start the server, write sample data and print the variables to export
#   scripts/test-influxdb.sh down   remove the container
#
# Load the variables with:  eval "$(scripts/test-influxdb.sh up)"
# Then:  cargo test -p dbclient-driver-influxdb
set -euo pipefail

NAME=dbclient-test-influxdb
# Pinned so a new server release cannot change what the tests see unnoticed.
IMAGE="${DBCLIENT_TEST_INFLUXDB_IMAGE:-influxdb:3.12.3-core}"

down() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

port() { docker port "$NAME" 8181 | head -1 | sed 's/.*://'; }

up() {
  down
  # Nothing is kept: the data lives in memory and disappears with the container.
  docker run -d --name "$NAME" -p 127.0.0.1::8181 "$IMAGE" \
    influxdb3 serve --node-id node0 --object-store memory >/dev/null
  local url token=""
  url="http://127.0.0.1:$(port)"
  # The first admin token is made by the server and shown once; it belongs to this container only.
  for _ in $(seq 1 60); do
    token=$(docker exec "$NAME" influxdb3 create token --admin --format json 2>/dev/null \
      | sed -n 's/.*"token": *"\([^"]*\)".*/\1/p' || true)
    [ -n "$token" ] && break
    sleep 1
  done
  if [ -z "$token" ]; then
    echo "timed out waiting for $NAME" >&2
    docker logs "$NAME" 2>&1 | tail -20 >&2
    return 1
  fi

  # `sensors.home`: 30 points a minute apart in two rooms; every fifth has no `note`.
  local lines="" i room city
  for i in $(seq 0 29); do
    if [ $((i % 2)) -eq 0 ]; then room=Mutfak city=İzmir; else room=Salon city=Ankara; fi
    local note=""
    [ $((i % 5)) -ne 0 ] && note=",note=\"Çağrı $i\""
    lines+="home,room=$room,city=$city temp=$((18 + i % 7)).5,hum=$((40 + i))i,ok=$([ $((i % 3)) -eq 0 ] && echo true || echo false)$note $((1700000000 + i * 60))"$'\n'
  done
  curl -fsS -X POST "$url/api/v3/write_lp?db=sensors&precision=s" \
    -H "Authorization: Bearer $token" --data-binary "$lines" >/dev/null
  curl -fsS -X POST "$url/api/v3/write_lp?db=garden&precision=s" \
    -H "Authorization: Bearer $token" --data-binary $'soil,bed=north moisture=0.31 1700000000\n' >/dev/null

  echo "export DBCLIENT_TEST_INFLUXDB_URL=$url"
  echo "export DBCLIENT_TEST_INFLUXDB_TOKEN=$token"
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
