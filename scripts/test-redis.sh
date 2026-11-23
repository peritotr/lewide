#!/usr/bin/env bash
# Starts a throwaway Redis server with a password for the driver tests.
#
#   scripts/test-redis.sh up     start it and print the variable the tests read
#   scripts/test-redis.sh tls    also start a second server that speaks TLS only, with a
#                                certificate signed by a generated test CA (optional;
#                                the TLS tests are skipped without it)
#   scripts/test-redis.sh down   remove both containers
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/test-redis"
NAME=dbclient-test-redis
NAME_TLS=dbclient-test-redis-tls
IMAGE=redis:7

port() { docker port "$1" 6379/tcp | head -1 | sed 's/.*://'; }

wait_for() {
  local name="$1"; shift
  for _ in $(seq 1 30); do
    docker exec "$name" redis-cli "$@" -a test --no-auth-warning ping 2>/dev/null | grep -q PONG && return 0
    sleep 1
  done
  echo "timed out waiting for $name" >&2
  docker logs "$name" 2>&1 | tail -20 >&2
  return 1
}

up() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  docker run -d --name "$NAME" -p 127.0.0.1::6379 "$IMAGE" redis-server --requirepass test >/dev/null
  wait_for "$NAME"
  echo "export DBCLIENT_TEST_REDIS_URL='redis://:test@127.0.0.1:$(port "$NAME")/0'"
}

make_certs() {
  rm -rf "$OUT" && mkdir -p "$OUT"
  cd "$OUT"
  cat > ca.cnf <<'CNF'
[req]
distinguished_name = dn
x509_extensions = v3_ca
prompt = no
[dn]
CN = dbclient test CA
[v3_ca]
basicConstraints = critical, CA:TRUE
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash
CNF
  # 192.0.2.10 is a documentation address nothing answers on. The tests use it
  # as the "real" host of a server that is only reachable through a route.
  cat > server.cnf <<'CNF'
[req]
distinguished_name = dn
prompt = no
[dn]
CN = dbclient test server
[v3_server]
basicConstraints = CA:FALSE
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = DNS:localhost, IP:127.0.0.1, IP:192.0.2.10
CNF
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config ca.cnf \
    -keyout ca.key -out ca.pem 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -config server.cnf \
    -keyout server.key -out server.csr 2>/dev/null
  openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 3650 \
    -extfile server.cnf -extensions v3_server -out server.crt 2>/dev/null
  # Readable by whichever user the server runs as inside the container.
  chmod 644 server.key
  cd "$ROOT"
}

tls() {
  docker rm -f "$NAME_TLS" >/dev/null 2>&1 || true
  make_certs
  # TLS only: the plain port is turned off. Clients need no certificate of their own.
  docker create --name "$NAME_TLS" -p 127.0.0.1::6379 "$IMAGE" redis-server \
    --requirepass test --port 0 --tls-port 6379 \
    --tls-cert-file /certs/server.crt --tls-key-file /certs/server.key \
    --tls-ca-cert-file /certs/ca.pem --tls-auth-clients no >/dev/null
  # Copied in rather than mounted, so no host path has to be shared.
  docker cp "$OUT" "$NAME_TLS:/certs" >/dev/null
  docker start "$NAME_TLS" >/dev/null
  wait_for "$NAME_TLS" --tls --cacert /certs/ca.pem
  echo "export DBCLIENT_TEST_REDIS_TLS_URL='redis://:test@127.0.0.1:$(port "$NAME_TLS")/0'"
  # Quoted: the path may contain spaces.
  echo "export DBCLIENT_TEST_REDIS_TLS_CA='$OUT/ca.pem'"
}

case "${1:-}" in
  up) up ;;
  tls) tls ;;
  down) docker rm -f "$NAME" "$NAME_TLS" >/dev/null 2>&1 || true ;;
  *) echo "usage: $0 up|tls|down" >&2; exit 2 ;;
esac
