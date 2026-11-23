#!/usr/bin/env bash
# Starts throwaway database servers for the driver tests:
#   PostgreSQL and MySQL with TLS (certificate signed by a generated test CA),
#   and an SSH server on the same Docker network for the tunnel tests.
#
#   scripts/test-servers.sh up     start everything, write target/test-servers/env.sh
#   scripts/test-servers.sh compat start MariaDB, CockroachDB, TiDB, YugabyteDB and TimescaleDB too (run after "up")
#   scripts/test-servers.sh down   remove the containers and the network
#
# Load the variables with:  . target/test-servers/env.sh
# (target/test-servers/env holds the same values as plain KEY=VALUE lines for CI.)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/test-servers"
NET=dbclient-test
PG=dbclient-test-pg
MYSQL=dbclient-test-mysql
SSH=dbclient-test-ssh
MARIADB=dbclient-test-mariadb
COCKROACH=dbclient-test-cockroach
TIDB=dbclient-test-tidb
YUGABYTE=dbclient-test-yugabyte
TIMESCALE=dbclient-test-timescale

down() {
  docker rm -f "$PG" "$MYSQL" "$SSH" "$MARIADB" "$COCKROACH" "$TIDB" "$YUGABYTE" "$TIMESCALE" >/dev/null 2>&1 || true
  docker network rm "$NET" >/dev/null 2>&1 || true
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
subjectAltName = DNS:localhost, DNS:pg, DNS:mysql, IP:127.0.0.1
CNF
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config ca.cnf \
    -keyout ca.key -out ca.pem 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -config server.cnf \
    -keyout server.key -out server.csr 2>/dev/null
  openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 3650 \
    -extfile server.cnf -extensions v3_server -out server.crt 2>/dev/null
  chmod 644 server.key

  ssh-keygen -q -t ed25519 -N "" -C dbclient-test -f "$OUT/ssh_key"
  cp "$OUT/ssh_key" "$OUT/ssh_key_encrypted"
  ssh-keygen -q -p -N "key-passphrase" -f "$OUT/ssh_key_encrypted"
  cd "$ROOT"
}

port() { docker port "$1" "$2" | head -1 | sed 's/.*://'; }

wait_for() {
  local name="$1"; shift
  for _ in $(seq 1 90); do
    if "$@" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "timed out waiting for $name" >&2
  docker logs "$name" 2>&1 | tail -20 >&2
  return 1
}

up() {
  down
  make_certs
  docker network create "$NET" >/dev/null

  # Certificates are copied in rather than mounted, so no host path has to be shared.
  docker create --name "$PG" --network "$NET" --network-alias pg \
    -e POSTGRES_PASSWORD=test -p 127.0.0.1::5432 --entrypoint sh postgres:17-alpine -c '
      cp /certs/server.crt /certs/server.key /var/lib/postgresql/ &&
      chown postgres /var/lib/postgresql/server.* &&
      chmod 600 /var/lib/postgresql/server.key &&
      exec docker-entrypoint.sh postgres -c ssl=on \
        -c ssl_cert_file=/var/lib/postgresql/server.crt \
        -c ssl_key_file=/var/lib/postgresql/server.key' >/dev/null
  docker cp "$OUT/certs" "$PG:/certs" >/dev/null
  docker start "$PG" >/dev/null

  docker create --name "$MYSQL" --network "$NET" --network-alias mysql \
    -e MYSQL_ROOT_PASSWORD=test -e MYSQL_DATABASE=app -p 127.0.0.1::3306 mysql:8.4 \
    --ssl-ca=/certs/ca.pem --ssl-cert=/certs/server.crt --ssl-key=/certs/server.key >/dev/null
  docker cp "$OUT/certs" "$MYSQL:/certs" >/dev/null
  docker start "$MYSQL" >/dev/null

  docker run -d --name "$SSH" --network "$NET" -p 127.0.0.1::22 \
    -e PUBKEY="$(cat "$OUT/ssh_key.pub")" alpine:3 sh -c '
      apk add --no-cache openssh-server >/dev/null &&
      ssh-keygen -A >/dev/null &&
      # The tests fail logins and drop handshakes on purpose; without these limits
      # lifted, sshd starts refusing the test machine.
      echo "root:test" | chpasswd 2>/dev/null &&
      mkdir -p /root/.ssh && echo "$PUBKEY" > /root/.ssh/authorized_keys &&
      chmod 700 /root/.ssh && chmod 600 /root/.ssh/authorized_keys &&
      exec /usr/sbin/sshd -D -e -o PermitRootLogin=yes -o PasswordAuthentication=yes \
        -o AllowTcpForwarding=yes -o MaxStartups=200 -o MaxSessions=200 \
        -o PerSourcePenalties=no' >/dev/null

  wait_for "$PG" docker exec "$PG" psql -U postgres -h 127.0.0.1 -c 'select 1'
  wait_for "$MYSQL" docker exec "$MYSQL" mysql -h 127.0.0.1 -uroot -ptest -e 'select 1' app
  wait_for "$SSH" sh -c "docker logs $SSH 2>&1 | grep -q 'Server listening'"

  cat > "$OUT/env" <<ENV
DBCLIENT_TEST_POSTGRES_URL=postgres://postgres:test@127.0.0.1:$(port "$PG" 5432)/postgres
DBCLIENT_TEST_MYSQL_URL=mysql://root:test@127.0.0.1:$(port "$MYSQL" 3306)/app
DBCLIENT_TEST_TLS_CA=$OUT/certs/ca.pem
DBCLIENT_TEST_SSH_ADDR=127.0.0.1:$(port "$SSH" 22)
DBCLIENT_TEST_SSH_HOST_KEY=$(docker exec "$SSH" cut -d' ' -f1,2 /etc/ssh/ssh_host_ed25519_key.pub)
DBCLIENT_TEST_SSH_USER=root
DBCLIENT_TEST_SSH_PASSWORD=test
DBCLIENT_TEST_SSH_KEY=$OUT/ssh_key
DBCLIENT_TEST_SSH_KEY_ENCRYPTED=$OUT/ssh_key_encrypted
DBCLIENT_TEST_SSH_KEY_PASSPHRASE=key-passphrase
ENV
  # Quoted form for shells: the paths may contain spaces.
  sed -e "s/'/'\\\\''/g" -e "s/^\([A-Z_]*\)=\(.*\)$/export \1='\2'/" "$OUT/env" > "$OUT/env.sh"
  echo "servers are up; load the variables with: . $OUT/env.sh" >&2
}

# MariaDB and TiDB speak the MySQL protocol; CockroachDB, YugabyteDB and
# TimescaleDB the PostgreSQL one. The driver suites run against them too, to
# show the drivers serve those engines:
#   DBCLIENT_TEST_MYSQL_URL=$DBCLIENT_TEST_MARIADB_URL cargo test -p dbclient-driver-mysql --test live
#   DBCLIENT_TEST_MYSQL_URL=$DBCLIENT_TEST_TIDB_URL cargo test -p dbclient-driver-mysql --test live
#   DBCLIENT_TEST_POSTGRES_URL=$DBCLIENT_TEST_COCKROACH_URL cargo test -p dbclient-driver-postgres --test live -- --test-threads=1
#   DBCLIENT_TEST_POSTGRES_URL=$DBCLIENT_TEST_YUGABYTE_URL cargo test -p dbclient-driver-postgres --test live -- --test-threads=1
#   DBCLIENT_TEST_POSTGRES_URL=$DBCLIENT_TEST_TIMESCALE_URL cargo test -p dbclient-driver-postgres --test live
# (CockroachDB and YugabyteDB reject schema changes that overlap, so their runs are not parallel.)
compat() {
  [ -f "$OUT/env" ] || { echo "run \"$0 up\" first" >&2; exit 1; }
  docker rm -f "$MARIADB" "$COCKROACH" "$TIDB" "$YUGABYTE" "$TIMESCALE" >/dev/null 2>&1 || true
  docker run -d --name "$MARIADB" -e MARIADB_ROOT_PASSWORD=test -e MARIADB_DATABASE=app \
    -p 127.0.0.1::3306 mariadb:11 >/dev/null
  # No authentication and no TLS: for tests on this machine only.
  docker run -d --name "$COCKROACH" -p 127.0.0.1::26257 cockroachdb/cockroach:latest \
    start-single-node --insecure >/dev/null
  # A single TiDB server with its built-in storage; root starts without a password.
  docker run -d --name "$TIDB" -p 127.0.0.1::4000 pingcap/tidb:v8.5.5 >/dev/null
  # yugabyted turns authentication on when YSQL_PASSWORD is set. Capped at 4 GB
  # (2 GB was killed for memory during the suite). On a busy machine its
  # clock checks fail and it stops itself, so they are relaxed. It also stops
  # when the disk is nearly full.
  docker run -d --name "$YUGABYTE" --memory=4g -p 127.0.0.1::5433 \
    -e YSQL_PASSWORD=test yugabytedb/yugabyte:2025.2.0.0-b131 \
    bin/yugabyted start --background=false \
    --master_flags=max_clock_skew_usec=5000000 \
    --tserver_flags=max_clock_skew_usec=5000000 >/dev/null
  docker run -d --name "$TIMESCALE" -e POSTGRES_PASSWORD=test -p 127.0.0.1::5432 \
    timescale/timescaledb:2.25.0-pg17 >/dev/null
  wait_for "$MARIADB" docker exec "$MARIADB" mariadb -uroot -ptest -e 'select 1' app
  wait_for "$COCKROACH" docker exec "$COCKROACH" ./cockroach sql --insecure -e 'select 1'
  wait_for "$TIDB" docker run --rm --network container:"$TIDB" mysql:8.4 \
    mysql -h 127.0.0.1 -P 4000 -uroot -e 'select 1'
  docker run --rm --network container:"$TIDB" mysql:8.4 mysql -h 127.0.0.1 -P 4000 -uroot \
    -e "ALTER USER 'root'@'%' IDENTIFIED BY 'test'; CREATE DATABASE IF NOT EXISTS app" >/dev/null
  wait_for "$YUGABYTE" docker exec -e PGPASSWORD=test "$YUGABYTE" \
    bin/ysqlsh -h "$(docker exec "$YUGABYTE" hostname)" -U yugabyte -c 'select 1'
  wait_for "$TIMESCALE" docker exec "$TIMESCALE" psql -U postgres -h 127.0.0.1 -c 'select 1'

  grep -v -e MARIADB -e COCKROACH -e TIDB -e YUGABYTE -e TIMESCALE "$OUT/env" > "$OUT/env.tmp"
  cat >> "$OUT/env.tmp" <<ENV
DBCLIENT_TEST_MARIADB_URL=mysql://root:test@127.0.0.1:$(port "$MARIADB" 3306)/app
DBCLIENT_TEST_COCKROACH_URL=postgres://root@127.0.0.1:$(port "$COCKROACH" 26257)/defaultdb
DBCLIENT_TEST_TIDB_URL=mysql://root:test@127.0.0.1:$(port "$TIDB" 4000)/app
DBCLIENT_TEST_YUGABYTE_URL=postgres://yugabyte:test@127.0.0.1:$(port "$YUGABYTE" 5433)/yugabyte
DBCLIENT_TEST_TIMESCALE_URL=postgres://postgres:test@127.0.0.1:$(port "$TIMESCALE" 5432)/postgres
ENV
  mv "$OUT/env.tmp" "$OUT/env"
  sed -e "s/'/'\\\\''/g" -e "s/^\([A-Z_]*\)=\(.*\)$/export \1='\2'/" "$OUT/env" > "$OUT/env.sh"
  echo "MariaDB, CockroachDB, TiDB, YugabyteDB and TimescaleDB are up; variables added to $OUT/env.sh" >&2
}

case "${1:-}" in
  up) up ;;
  compat) compat ;;
  down) down ;;
  *) echo "usage: $0 up|compat|down" >&2; exit 2 ;;
esac
