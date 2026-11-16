#!/usr/bin/env bash
# Point-in-time restore drill, in an isolated compose project.
#
#   down   → stop (and clear) the isolated instance
#   pitr   → pgBackRest restores the base backup and replays WAL to a target
#   up     → the isolated PostgreSQL recovers, promotes and is verified
#
# The running instance is never touched.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"
start_log restore

PROJECT="oxide-restore"
RESTORE_COMPOSE=(docker compose -p "$PROJECT" -f docker-compose.restore.yml)
DB_USER="${POSTGRES_USER:-oxide}"
DB_NAME="${POSTGRES_DB:-oxide}"
BACKUP_BUCKET="${S3_BACKUP_BUCKET:-oxide-backup}"
S3_ARGS=(
  --s3
  --s3-bucket "${BACKUP_BUCKET}"
  --s3-region us-east-1
  --s3-access-key-id "${S3_ACCESS_KEY_ID:?run cp .env.example .env}"
  --s3-secret-access-key "${S3_SECRET_ACCESS_KEY:?run cp .env.example .env}"
  --s3-endpoint http://minio:9000
)
started_ms=$(now_ms)

echo "-- pausing the line so the source is stable"
api_post /api/sim/pause >/dev/null || true
wait_snapshot 'd["db"]["outbox_pending"] == 0' 120 || { echo "FAIL: outbox did not drain"; exit 1; }

echo "-- source snapshot"
snapshot=$(api_get /api/snapshot)
lots=$(echo "$snapshot" | json db.lots)
events=$(echo "$snapshot" | json db.events)
max_created=$(docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  "SELECT COALESCE(to_char(max(created_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), 'none') FROM lot")
main_greptime=$(api_get /api/stats | python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["greptime"]))')
echo "   lots=${lots} events=${events} max(created_at)=${max_created}"

echo "-- marking a restore point and archiving WAL past it"
# A time target must lie inside the archived WAL, so the marker transaction is
# written first, then real WAL is generated and switched into the repository.
target_time=$(docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAq -c \
  "BEGIN; SELECT pg_create_restore_point('oxide_drill'); SELECT to_char(clock_timestamp() AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US') || '+00'; COMMIT;" \
  | tail -1 | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
echo "   PITR target ${target_time}"
docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -q -c \
  "DROP TABLE IF EXISTS wal_probe; CREATE TABLE wal_probe AS SELECT generate_series(1, 20000) AS n, repeat('x', 1024) AS pad;"
docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc 'SELECT pg_switch_wal()' >/dev/null
before_wal=$(docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  "SELECT COALESCE(last_archived_wal, 'none') FROM pg_stat_archiver")
for _ in $(seq 1 60); do
  now_wal=$(docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
    "SELECT COALESCE(last_archived_wal, 'none') FROM pg_stat_archiver")
  [ "$now_wal" != "$before_wal" ] && [ "$now_wal" != "none" ] && break
  sleep 1
done
docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -q -c "DROP TABLE IF EXISTS wal_probe;"
echo "   WAL archived through ${now_wal}"

echo "-- down: clearing the isolated instance"
"${RESTORE_COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
"${RESTORE_COMPOSE[@]}" create >/dev/null

echo "-- pitr: pgBackRest restore into the isolated volume"
"${RESTORE_COMPOSE[@]}" run --rm --no-deps --entrypoint sh postgres -c "
  set -e
  rm -rf /var/lib/postgresql/data/*
  pgbackrest --stanza=oxide --type=time --target='${target_time}' --target-action=promote \
    --log-level-console=info restore
" | tee "$ARTIFACTS/restore-pgbackrest-${TS}.log" | tail -12

echo "-- up: starting the isolated PostgreSQL and GreptimeDB"
"${RESTORE_COMPOSE[@]}" up -d --wait postgres greptimedb

echo "-- waiting for recovery to finish and promote"
recovered="f"
for _ in $(seq 1 120); do
  recovered=$("${RESTORE_COMPOSE[@]}" exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
    'SELECT pg_is_in_recovery()' 2>/dev/null | tr -d '[:space:]' || echo "t")
  [ "$recovered" = "f" ] && break
  sleep 1
done
if [ "$recovered" != "f" ]; then
  echo "FAIL: the isolated instance never left recovery"
  "${RESTORE_COMPOSE[@]}" logs postgres | tail -30
  exit 1
fi

recovered_at=$(now_utc)
wal_lsn=$("${RESTORE_COMPOSE[@]}" exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  'SELECT pg_last_wal_replay_lsn()' | tr -d '[:space:]')
replay_ts=$("${RESTORE_COMPOSE[@]}" exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  "SELECT COALESCE(to_char(pg_last_xact_replay_timestamp() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), 'none')" | tr -d '[:space:]')
restored_lots=$("${RESTORE_COMPOSE[@]}" exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  'SELECT count(*) FROM lot' | tr -d '[:space:]')
restored_events=$("${RESTORE_COMPOSE[@]}" exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  'SELECT count(*) FROM lot_event' | tr -d '[:space:]')
restored_max_created=$("${RESTORE_COMPOSE[@]}" exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  "SELECT COALESCE(to_char(max(created_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), 'none') FROM lot" | tr -d '[:space:]')
restored_max_event=$("${RESTORE_COMPOSE[@]}" exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  "SELECT COALESCE(to_char(max(occurred_at) AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), 'none') FROM lot_event" | tr -d '[:space:]')

echo "   is_in_recovery=f lsn=${wal_lsn} last_xact_replay=${replay_ts}"
echo "   lots=${restored_lots} events=${restored_events}"
echo "   max(created_at)=${restored_max_created} max(occurred_at)=${restored_max_event}"

failures=()
[ "$restored_lots" -ge "$lots" ] || failures+=("restored lots ${restored_lots} < ${lots}")
[ "$restored_events" -ge "$events" ] || failures+=("restored events ${restored_events} < ${events}")
[ "$restored_max_created" != "none" ] || failures+=("restored lot table is empty")
if [ "$restored_max_created" != "$max_created" ]; then
  failures+=("max(created_at) ${restored_max_created} != ${max_created}")
fi

if [ ${#failures[@]} -gt 0 ]; then
  echo "FAIL: ${failures[*]}"
  exit 1
fi

echo "-- GreptimeDB: importing the Export/Import V2 snapshot into the isolated instance"
snapshot_uri=$(api_get /api/backup | json greptime_snapshot)
if [ -z "$snapshot_uri" ] || [ "$snapshot_uri" = "None" ]; then
  echo "FAIL: no GreptimeDB snapshot in the last backup; run scripts/backup.sh first"
  exit 1
fi
"${RESTORE_COMPOSE[@]}" exec -T greptimedb \
  greptime cli data import-v2 --addr 127.0.0.1:4000 --from "$snapshot_uri" "${S3_ARGS[@]}" \
  | tee "$ARTIFACTS/restore-greptime-${TS}.log" | tail -6

restored_greptime=$("${RESTORE_COMPOSE[@]}" exec -T greptimedb \
  curl -sS -X POST -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode 'sql=SELECT count(*) AS c FROM equipment_telemetry' \
  'http://127.0.0.1:4000/v1/sql?db=public')
echo "   isolated equipment_telemetry: ${restored_greptime}"

greptime_tables=$(echo "$main_greptime" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')
restored_rows=$(echo "$restored_greptime" | python3 -c '
import json, sys
body = json.load(sys.stdin)
rows = body["output"][0]["records"]["rows"]
print(int(rows[0][0]) if rows else 0)')
if [ "$restored_rows" -le 0 ]; then
  echo "FAIL: the isolated GreptimeDB has no telemetry after the import"
  exit 1
fi

elapsed_ms=$(( $(now_ms) - started_ms ))
echo "-- isolated instance verified, tearing it down"
"${RESTORE_COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true

python3 - "$TS" "$PROJECT" "$target_time" "$recovered_at" "$restored_lots" "$restored_events" \
  "$restored_max_created" "$restored_max_event" "$greptime_tables" "$restored_rows" \
  "$elapsed_ms" "$LOG" > "$ARTIFACTS/restore-latest.json" <<'PY'
import json
import sys

(ts, project, target, recovered_at, lots, events, max_created, max_event,
 tables, rows, elapsed_ms, log) = sys.argv[1:13]
print(json.dumps({
    "drilled_at": ts,
    "project": project,
    "target_time": target,
    "recovered_at": recovered_at,
    "lots": int(lots),
    "events": int(events),
    "max_created_at": max_created,
    "max_event_at": max_event,
    "greptime_tables": int(tables),
    "greptime_rows": int(rows),
    "elapsed_ms": int(elapsed_ms),
    "ok": True,
    "log_path": log,
}, indent=2))
PY
cat "$ARTIFACTS/restore-latest.json"
api_post /admin/report/restore "$(cat "$ARTIFACTS/restore-latest.json")" >/dev/null
api_post /api/sim/play >/dev/null || true
echo "PASS: point-in-time restore verified in the isolated project (${elapsed_ms} ms)"
