#!/usr/bin/env bash
# Starts a throwaway Qdrant server for the driver tests.
#
#   scripts/test-qdrant.sh up     start the server and print the variables to export
#   scripts/test-qdrant.sh tls    also start a second server that speaks TLS, with a
#                                 certificate signed by a generated test CA (optional;
#                                 the TLS tests are skipped without it)
#   scripts/test-qdrant.sh down   remove both containers
#
# Load the variables with:  eval "$(scripts/test-qdrant.sh up)"
# Then:  cargo test -p dbclient-driver-qdrant
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/test-qdrant"
NAME=dbclient-test-qdrant
NAME_TLS=dbclient-test-qdrant-tls
# Pinned so a new server release cannot change what the tests see unnoticed.
IMAGE="${DBCLIENT_TEST_QDRANT_IMAGE:-qdrant/qdrant:v1.20.2}"
# A made-up key for these containers only. With a key set, the tests can also
# check that a wrong one is refused.
API_KEY=dbclient-test-key

down() {
  docker rm -f "$NAME" "$NAME_TLS" >/dev/null 2>&1 || true
}

port() { docker port "$1" 6333 | head -1 | sed 's/.*://'; }

# /readyz needs no key and answers 200 once collections can be served.
wait_for() {
  local name="$1" url="$2"
  for _ in $(seq 1 60); do
    if curl -fsSk -o /dev/null "$url/readyz" 2>/dev/null; then return 0; fi
    sleep 1
  done
  echo "timed out waiting for $name" >&2
  docker logs "$name" 2>&1 | tail -20 >&2
  return 1
}

up() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  # Nothing is mounted: the data disappears with the container.
  docker run -d --name "$NAME" -p 127.0.0.1::6333 \
    -e QDRANT__SERVICE__API_KEY="$API_KEY" \
    -e QDRANT__TELEMETRY_DISABLED=true \
    "$IMAGE" >/dev/null
  local url
  url="http://127.0.0.1:$(port "$NAME")"
  wait_for "$NAME" "$url"
  echo "export DBCLIENT_TEST_QDRANT_URL=$url"
  echo "export DBCLIENT_TEST_QDRANT_API_KEY=$API_KEY"
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
  # The certificate is copied in rather than mounted, so no host path has to be shared.
  docker create --name "$NAME_TLS" -p 127.0.0.1::6333 \
    -e QDRANT__SERVICE__API_KEY="$API_KEY" \
    -e QDRANT__TELEMETRY_DISABLED=true \
    -e QDRANT__SERVICE__ENABLE_TLS=true \
    -e QDRANT__TLS__CERT=/certs/server.crt \
    -e QDRANT__TLS__KEY=/certs/server.key \
    "$IMAGE" >/dev/null
  docker cp "$OUT" "$NAME_TLS:/certs" >/dev/null
  docker start "$NAME_TLS" >/dev/null
  local url
  url="https://127.0.0.1:$(port "$NAME_TLS")"
  wait_for "$NAME_TLS" "$url"
  echo "export DBCLIENT_TEST_QDRANT_TLS_URL=$url"
  # Quoted: the path may contain spaces.
  echo "export DBCLIENT_TEST_QDRANT_TLS_CA='$OUT/ca.pem'"
  echo "export DBCLIENT_TEST_QDRANT_API_KEY=$API_KEY"
}

case "${1:-}" in
  up) up ;;
  tls) tls ;;
  down) down ;;
  *) echo "usage: $0 up|tls|down" >&2; exit 2 ;;
esac
