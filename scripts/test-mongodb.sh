#!/usr/bin/env bash
# Starts a throwaway MongoDB server for the driver tests.
#
#   scripts/test-mongodb.sh up     start the container, print the variables to export
#   scripts/test-mongodb.sh down   remove the container
#
# Use it as:  eval "$(scripts/test-mongodb.sh up)"
#             cargo test -p dbclient-driver-mongodb --test live
#
# The server accepts plain and TLS connections on the same port; its
# certificate is signed by a test CA generated here. The root password is for
# this container only, which listens on localhost and is removed by "down".
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/test-mongodb"
NAME=dbclient-test-mongodb
IMAGE="${DBCLIENT_TEST_MONGODB_IMAGE:-mongo:8.0}"
USER_NAME=root
PASSWORD=test

down() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

make_certs() {
  rm -rf "$OUT" && mkdir -p "$OUT/certs"
  cd "$OUT/certs"
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
  # mongo.dbclient.test resolves nowhere: the tests use it to show that a
  # routed connection checks the certificate against the name, not the address.
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
subjectAltName = DNS:localhost, DNS:mongo.dbclient.test, IP:127.0.0.1
CNF
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config ca.cnf \
    -keyout ca.key -out ca.pem 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -config server.cnf \
    -keyout server.key -out server.csr 2>/dev/null
  openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 3650 \
    -extfile server.cnf -extensions v3_server -out server.crt 2>/dev/null
  # mongod wants certificate and key in one file, readable by its own user.
  cat server.crt server.key > server.pem
  chmod 644 server.pem
  cd "$ROOT"
}

up() {
  down
  make_certs

  # The certificate is copied in rather than mounted, so no host path has to be shared.
  # mongod refuses TLS without a CA file even when, as here, clients present no certificate.
  docker create --name "$NAME" -p 127.0.0.1::27017 \
    -e MONGO_INITDB_ROOT_USERNAME="$USER_NAME" -e MONGO_INITDB_ROOT_PASSWORD="$PASSWORD" \
    "$IMAGE" --tlsMode allowTLS --tlsCertificateKeyFile /certs/server.pem \
    --tlsCAFile /certs/ca.pem --tlsAllowConnectionsWithoutCertificates >/dev/null
  docker cp "$OUT/certs" "$NAME:/certs" >/dev/null
  docker start "$NAME" >/dev/null

  # While the image creates the root user it runs a first server that listens
  # on the container's loopback only. Asking on the container's own address
  # waits for the real one.
  local ready=0
  for _ in $(seq 1 90); do
    if docker exec "$NAME" sh -c 'mongosh --quiet --host "$(hostname -i)" \
        -u "$MONGO_INITDB_ROOT_USERNAME" -p "$MONGO_INITDB_ROOT_PASSWORD" \
        --authenticationDatabase admin --eval "db.runCommand({ ping: 1 }).ok"' >/dev/null 2>&1; then
      ready=1
      break
    fi
    sleep 1
  done
  if [ "$ready" != 1 ]; then
    echo "timed out waiting for $NAME" >&2
    docker logs "$NAME" 2>&1 | tail -20 >&2
    exit 1
  fi

  local port
  port="$(docker port "$NAME" 27017 | head -1 | sed 's/.*://')"
  echo "MongoDB ($IMAGE) is up on 127.0.0.1:$port; stop it with: $0 down" >&2
  echo "export DBCLIENT_TEST_MONGODB_URL='mongodb://$USER_NAME:$PASSWORD@127.0.0.1:$port/'"
  # Quoted for shells: the path may contain spaces.
  echo "export DBCLIENT_TEST_MONGODB_CA='$(printf '%s' "$OUT/certs/ca.pem" | sed "s/'/'\\\\''/g")'"
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
