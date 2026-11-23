#!/usr/bin/env bash
# Starts a throwaway SQL Server (Developer edition) for the driver tests.
# The image is x86 only; on Apple silicon Docker runs it through Rosetta,
# and the first start takes a minute or two.
#
# Starting it accepts Microsoft's licence terms for this container
# (ACCEPT_EULA=Y). The user agreed to that for local test containers.
#
#   scripts/test-sqlserver.sh up     start it and print the variable the tests read
#   scripts/test-sqlserver.sh down   remove the container
set -euo pipefail

NAME=dbclient-test-sqlserver
IMAGE="${SQLSERVER_IMAGE:-mcr.microsoft.com/mssql/server:2022-latest}"
# Only for this container. SQL Server refuses passwords without upper case,
# lower case, a digit and a symbol.
PASSWORD='Test-pass1'

case "${1:-}" in
  up)
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --name "$NAME" --platform linux/amd64 \
      -e ACCEPT_EULA=Y -e MSSQL_SA_PASSWORD="$PASSWORD" -e MSSQL_PID=Developer \
      `# Left alone it takes most of Docker's memory and is the first to be killed when that runs out.` \
      -e MSSQL_MEMORY_LIMIT_MB=2048 \
      -p 127.0.0.1::1433 "$IMAGE" >/dev/null
    ready=""
    for _ in $(seq 1 120); do
      if docker exec "$NAME" /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P "$PASSWORD" -Q "SELECT 1" >/dev/null 2>&1; then
        ready=1
        break
      fi
      sleep 2
    done
    if [ -z "$ready" ]; then
      echo "SQL Server did not start; see: docker logs $NAME" >&2
      exit 1
    fi
    docker exec "$NAME" /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P "$PASSWORD" \
      -Q "IF DB_ID('dbclient_test') IS NULL CREATE DATABASE dbclient_test" >/dev/null
    PORT="$(docker port "$NAME" 1433/tcp | head -1 | sed 's/.*://')"
    echo "export DBCLIENT_TEST_SQLSERVER_URL='sqlserver://sa:$PASSWORD@127.0.0.1:$PORT/dbclient_test'"
    ;;
  down) docker rm -f "$NAME" >/dev/null 2>&1 || true ;;
  *) echo "usage: $0 up|down" >&2; exit 2 ;;
esac
