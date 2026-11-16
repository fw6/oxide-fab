#!/usr/bin/env bash
# Run one SQL statement against GreptimeDB through its HTTP interface.
#
#   ./scripts/greptime-sql.sh 'SHOW TABLES'
#
# The query runs inside the greptimedb container, so no host port is needed.
set -euo pipefail

SQL="${1:?usage: greptime-sql.sh \"<sql>\"}"
cd "$(dirname "$0")/.."

docker compose exec -T greptimedb \
  curl -sS -X POST \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode "sql=${SQL}" \
  'http://127.0.0.1:4000/v1/sql?db=public'
echo
