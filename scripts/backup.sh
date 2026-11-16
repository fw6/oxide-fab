#!/usr/bin/env bash
# One backup: a pgBackRest full base backup with WAL archiving to the
# S3-compatible object store's `oxide-backup` bucket, logical dumps beside it,
# and a GreptimeDB Export/Import V2 snapshot. Prints the inventory it wrote.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"
start_log backup

DB_USER="${POSTGRES_USER:-oxide}"
DB_NAME="${POSTGRES_DB:-oxide}"
BACKUP_BUCKET="${S3_BACKUP_BUCKET:-oxide-backup}"
SNAPSHOT="s3://${BACKUP_BUCKET}/greptime/${TS}"
S3_ARGS=(
  --s3
  --s3-bucket "${BACKUP_BUCKET}"
  --s3-region us-east-1
  --s3-access-key-id "${S3_ACCESS_KEY_ID:?run cp .env.example .env}"
  --s3-secret-access-key "${S3_SECRET_ACCESS_KEY:?run cp .env.example .env}"
  --s3-endpoint http://minio:9000
)

echo "-- PostgreSQL stanza"
docker compose exec -T -u postgres postgres \
  pgbackrest --stanza=oxide --log-level-console=warn stanza-create 2>/dev/null || true
docker compose exec -T -u postgres postgres \
  pgbackrest --stanza=oxide --log-level-console=info check | tee "$ARTIFACTS/pgbackrest-check-${TS}.log"

echo "-- WAL continuity: reset the counters, fill and switch a segment, watch it land"
docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  "SELECT pg_stat_reset_shared('archiver')" >/dev/null
# A switch alone archives nothing if that segment was already pushed, so make
# real WAL first: 20k rows of 1 KiB is well past the 16 MiB segment size.
docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -q -c \
  "DROP TABLE IF EXISTS wal_probe; CREATE TABLE wal_probe AS SELECT generate_series(1, 20000) AS n, repeat('x', 1024) AS pad;"
switched=$(docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc 'SELECT pg_switch_wal()')
echo "   switched WAL at ${switched}"
archive_state="1/0/none"
for _ in $(seq 1 60); do
  archive_state=$(docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -tAc \
    "SELECT archived_count || '/' || failed_count || '/' || COALESCE(last_archived_wal, 'none') FROM pg_stat_archiver")
  if [ "$(echo "$archive_state" | awk -F/ '{print $1}')" -ge 1 ]; then
    break
  fi
  sleep 1
done
echo "   archived/failed/last_archived_wal = ${archive_state}"
docker compose exec -T postgres psql -U "$DB_USER" -d "$DB_NAME" -q -c "DROP TABLE IF EXISTS wal_probe;"
failed_count=$(echo "$archive_state" | awk -F/ '{print $2}')
archived_count=$(echo "$archive_state" | awk -F/ '{print $1}')
if [ "${failed_count:-1}" != "0" ] || [ "${archived_count:-0}" -lt 1 ]; then
  echo "FAIL: WAL archiving is not continuous (${archive_state})"
  exit 1
fi

echo "-- full base backup into ${BACKUP_BUCKET}"
docker compose exec -T -u postgres postgres \
  pgbackrest --stanza=oxide --type=full --log-level-console=info backup \
  | tee "$ARTIFACTS/pgbackrest-backup-${TS}.log"

echo "-- pgBackRest inventory"
info=$(docker compose exec -T -u postgres postgres \
  pgbackrest --stanza=oxide info --output=json)
echo "$info" | tee "$ARTIFACTS/pgbackrest-info-${TS}.json" | python3 -c '
import json, sys
d = json.load(sys.stdin)
for stanza in d:
    for backup in stanza.get("backup", []):
        print("   backup {} type={} lsn={} size={}B".format(
            backup["label"], backup["type"],
            backup.get("lsn", {}).get("stop", "?"),
            backup.get("info", {}).get("repository", {}).get("delta", "?")))
    archive = stanza.get("archive", [])
    for segment in archive:
        print("   archive {} min={} max={}".format(
            segment.get("repo", "?"), segment.get("min"), segment.get("max")))
'
backup_label=$(echo "$info" | python3 -c '
import json, sys
d = json.load(sys.stdin)
labels = [b["label"] for s in d for b in s.get("backup", []) if b["type"] == "full"]
print(sorted(labels)[-1] if labels else "")')

echo "-- logical dumps"
pg_dump_file="pg-${TS}.dump"
globals_file="pg-globals-${TS}.sql"
docker compose exec -T postgres pg_dump -U "$DB_USER" -Fc "$DB_NAME" > "$ARTIFACTS/$pg_dump_file"
docker compose exec -T postgres pg_dumpall -U "$DB_USER" --globals-only > "$ARTIFACTS/$globals_file"
du -h "$ARTIFACTS/$pg_dump_file" "$ARTIFACTS/$globals_file" | sed 's/^/   /'
docker compose run --rm --entrypoint sh minio-init -c "
  set -e
  endpoint=http://minio:9000
  aws --endpoint-url \"\$endpoint\" s3 cp /artifacts/${pg_dump_file} s3://${BACKUP_BUCKET}/pg-logical/${TS}/${pg_dump_file} >/dev/null
  aws --endpoint-url \"\$endpoint\" s3 cp /artifacts/${globals_file} s3://${BACKUP_BUCKET}/pg-logical/${TS}/${globals_file} >/dev/null
  echo '   uploaded to s3://${BACKUP_BUCKET}/pg-logical/${TS}/'
"

echo "-- GreptimeDB export (Export/Import V2)"
docker compose exec -T greptimedb \
  greptime cli data export-v2 create --addr 127.0.0.1:4000 --to "$SNAPSHOT" \
  "${S3_ARGS[@]}" --schemas public
docker compose exec -T greptimedb \
  greptime cli data export-v2 verify --snapshot "$SNAPSHOT" "${S3_ARGS[@]}"

echo "-- inventory"
python3 - "$TS" "$backup_label" "$pg_dump_file" "$globals_file" "$SNAPSHOT" "$archive_state" "$LOG" > "$ARTIFACTS/backup-latest.json" <<'PY'
import json
import sys

ts, label, dump, globals_file, snapshot, archive_state, log = sys.argv[1:8]
print(json.dumps({
    "taken_at": ts,
    "pg_full_backup": label,
    "pg_logical_dump": dump,
    "pg_globals_dump": globals_file,
    "greptime_snapshot": snapshot,
    "wal_continuity": archive_state,
    "ok": True,
    "log_path": log,
}, indent=2))
PY
cat "$ARTIFACTS/backup-latest.json"
api_post /admin/report/backup "$(cat "$ARTIFACTS/backup-latest.json")" >/dev/null
echo "PASS: backup written to s3://${BACKUP_BUCKET}/ (PG base + logical dumps + Greptime snapshot)"
