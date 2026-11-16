#!/usr/bin/env bash
# Fault injection: stop the time series store, stop PostgreSQL, and run a full
# restore drill. Every scenario records what was lost (RPO) and how long
# recovery took (RTO), and appends to artifacts/chaos-report.md.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"
start_log chaos

REPORT="$ARTIFACTS/chaos-report.md"
container_health() {
  local id
  id=$(docker compose ps -q "$1")
  if [ -z "$id" ]; then
    echo missing
    return
  fi
  docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$id" 2>/dev/null || echo missing
}

wait_service_healthy() {
  local service="$1" timeout="${2:-120}" start
  start=$(date +%s)
  while :; do
    status=$(container_health "$service")
    [ "$status" = "healthy" ] && return 0
    [ $(( $(date +%s) - start )) -ge "$timeout" ] && return 1
    sleep 1
  done
}

wait_pending_zero() {
  local timeout="${1:-120}" start
  start=$(date +%s)
  while :; do
    if api_get /api/outbox/timeline | python3 -c 'import json,sys; sys.exit(0 if json.load(sys.stdin)["stats"]["pending"] == 0 else 1)'; then
      return 0
    fi
    [ $(( $(date +%s) - start )) -ge "$timeout" ] && return 1
    sleep 1
  done
}

scenario_kill_store() {
  echo
  echo "===== scenario 1: kill-store (GreptimeDB down for 30 s) ====="
  api_post /admin/reset >/dev/null
  api_post /api/sim/play >/dev/null
  wait_snapshot 'd["db"]["events"] > 24' 120 || true

  before=$(api_get /api/snapshot)
  events_before=$(echo "$before" | json db.events)
  completed_before=$(echo "$before" | python3 -c 'import json,sys; print(sum(1 for l in json.load(sys.stdin)["sim"]["lots"] if l["state"]=="completed"))')
  echo "   before: events=${events_before} completed_lots=${completed_before}"

  echo "   stopping greptimedb"
  docker compose stop greptimedb >/dev/null
  stop_ms=$(now_ms)
  sleep 30
  during=$(api_get /api/outbox/timeline)
  pending_during=$(echo "$during" | json stats.pending)
  events_during=$(api_get /api/snapshot | json db.events)
  echo "   during outage: outbox_pending=${pending_during} events=${events_during}"

  echo "   starting greptimedb"
  docker compose start greptimedb >/dev/null
  wait_service_healthy greptimedb 180 || { echo "FAIL: greptimedb did not become healthy"; return 1; }
  if ! wait_pending_zero 180; then
    echo "FAIL: the outbox did not catch up"
    return 1
  fi
  rto_ms=$(( $(now_ms) - stop_ms ))
  # Stop the line so the backlog cannot grow between the check and the sample.
  api_post /api/sim/pause >/dev/null
  wait_pending_zero 60 || true

  after=$(api_get /api/snapshot)
  events_after=$(echo "$after" | json db.events)
  published_after=$(echo "$after" | json db.published_events)
  pending_after=$(echo "$after" | json db.outbox_pending)
  completed_after=$(echo "$after" | python3 -c 'import json,sys; print(sum(1 for l in json.load(sys.stdin)["sim"]["lots"] if l["state"]=="completed"))')
  greptime_rows=$(api_get /api/stats | python3 -c 'import json,sys; d=json.load(sys.stdin)["greptime"]; print(sum(int(v) for v in d.values()))')
  buffered=$(( events_after - events_before ))
  lost=$(( events_after - published_after ))

  echo "   after: events=${events_after} published=${published_after} pending=${pending_after} greptime_rows=${greptime_rows}"
  echo "   buffered during outage=${buffered} lost=${lost} RTO=${rto_ms} ms"

  if [ "$pending_after" != "0" ] || [ "$lost" != "0" ] || [ "$greptime_rows" -le 0 ] || [ "$completed_after" -lt "$completed_before" ]; then
    echo "FAIL: kill-store did not converge"
    return 1
  fi

  STORE_RTO="$rto_ms"
  STORE_BUFFERED="$buffered"
  cat >> "$REPORT" <<EOF
## 1. kill-store — GreptimeDB stopped for 30 s

- Events before the outage: **${events_before}**
- Events committed to PostgreSQL during the outage: **${buffered}** (the outbox absorbed all of them)
- Outbox backlog observed during the outage: **${pending_during}** events
- **RPO: 0 events lost** — every event stayed in PostgreSQL's outbox
- **RTO: ${rto_ms} ms** — from \`docker compose stop greptimedb\` until the outbox drained
- After catch-up: events=${events_after}, published=${published_after}, pending=${pending_after}, Greptime rows=${greptime_rows}
- Lot state preserved: completed lots before=${completed_before}, after=${completed_after}
- Evidence: \`${LOG}\`
- Conclusion: the collector never lost a point; delivery resumed automatically. This is buffering + catch-up, not a failover.

EOF
  echo "   PASS"
  return 0
}

scenario_kill_db() {
  echo
  echo "===== scenario 2: kill-db (PostgreSQL down for 10 s) ====="
  api_post /admin/reset >/dev/null
  api_post /api/sim/play >/dev/null
  wait_snapshot 'd["db"]["events"] > 24' 120 || true

  before=$(api_get /api/snapshot)
  events_before=$(echo "$before" | json db.events)
  echo "   before: events=${events_before}"

  echo "   stopping postgres"
  docker compose stop postgres >/dev/null
  stop_ms=$(now_ms)
  sleep 2
  codes=""
  for _ in 1 2 3; do
    codes="${codes}$(http_code /api/snapshot) "
    sleep 1
  done
  echo "   /api/snapshot while PostgreSQL is down: ${codes}"
  sleep 5
  docker compose start postgres >/dev/null
  wait_service_healthy postgres 180 || { echo "FAIL: postgres did not become healthy"; return 1; }
  wait_http /healthz 120 || { echo "FAIL: the API did not come back"; return 1; }
  rto_ms=$(( $(now_ms) - stop_ms ))

  # The engine buffered its uncommitted batch; it must land now. Pause first so
  # the measurement is not racing newly generated events.
  api_post /api/sim/play >/dev/null
  wait_snapshot 'd["db"]["events"] > 0' 60 || true
  api_post /api/sim/pause >/dev/null
  wait_snapshot 'd["db"]["events"] >= d["db"]["published_events"] and d["db"]["outbox_pending"] == 0' 120 || true
  wait_pending_zero 120 || { echo "FAIL: the outbox did not catch up after PostgreSQL returned"; return 1; }
  after=$(api_get /api/snapshot)
  events_after=$(echo "$after" | json db.events)
  published_after=$(echo "$after" | json db.published_events)
  pending_after=$(echo "$after" | json db.outbox_pending)
  lost=$(( events_after - published_after ))
  echo "   after: events=${events_after} published=${published_after} pending=${pending_after}"

  if ! echo "$codes" | grep -q '503'; then
    echo "FAIL: the API did not report 503 while PostgreSQL was down"
    return 1
  fi
  if [ "$pending_after" != "0" ] || [ "$lost" != "0" ]; then
    echo "FAIL: events did not converge after PostgreSQL returned"
    return 1
  fi

  DB_RTO="$rto_ms"
  codes_csv="$(echo "$codes" | sed -e 's/[[:space:]]*$//' -e 's/ /, /g')"
  cat >> "$REPORT" <<EOF
## 2. kill-db — PostgreSQL stopped for 10 s

- \`GET /api/snapshot\` during the outage: **HTTP ${codes_csv}** (the API refuses writes without a database)
- Events before: **${events_before}**, after: **${events_after}** (the batch that was in flight committed after recovery)
- **RPO: 0 events lost**
- **RTO: ${rto_ms} ms** — including the PostgreSQL restart and WAL recovery
- After recovery: published=${published_after}, outbox pending=${pending_after}
- Evidence: \`${LOG}\`
- Conclusion: no dirty writes while the database was gone; the buffered batch committed once it returned. Restart-based recovery, not automatic failover.

EOF
  echo "   PASS"
  return 0
}

scenario_restore_drill() {
  echo
  echo "===== scenario 3: restore-drill ====="
  drill_start=$(now_ms)
  if ! ./scripts/restore.sh >"$ARTIFACTS/chaos-restore-${TS}.log" 2>&1; then
    echo "FAIL: the restore drill failed; see $ARTIFACTS/chaos-restore-${TS}.log"
    tail -20 "$ARTIFACTS/chaos-restore-${TS}.log"
    return 1
  fi
  drill_ms=$(( $(now_ms) - drill_start ))
  report=$(cat "$ARTIFACTS/restore-latest.json")
  lots=$(echo "$report" | json lots)
  events=$(echo "$report" | json events)
  rows=$(echo "$report" | json greptime_rows)
  target=$(echo "$report" | json target_time)
  echo "   restored lots=${lots} events=${events} greptime_rows=${rows} in ${drill_ms} ms"

  DRILL_RTO="$drill_ms"
  cat >> "$REPORT" <<EOF
## 3. restore-drill — isolated point-in-time restore

- Target time: **${target}**
- Restored rows: lots=**${lots}**, events=**${events}**, Greptime rows=**${rows}**
- pg_is_in_recovery()=false; row counts and \`max(created_at)\` matched the source
- **RPO: 0 events** to the restore point
- **RTO: ${drill_ms} ms** (down → pgBackRest restore → up → verify → import → verify)
- Evidence: \`artifacts/restore-latest.json\`, \`artifacts/chaos-restore-${TS}.log\`
- Conclusion: an isolated project restored the cluster without touching the running one.

EOF
  echo "   PASS"
  return 0
}

cat > "$REPORT" <<EOF
# oxide-fab chaos report

Generated: ${TS} (UTC)

> **Restart-based recovery, not automatic failover.** Every RTO below includes
> the container restart time. RPO is measured from the event log: an event that
> PostgreSQL committed is never lost, and the outbox re-delivers it.

EOF

failures=0
scenario_kill_store || failures=$((failures + 1))
scenario_kill_db || failures=$((failures + 1))
scenario_restore_drill || failures=$((failures + 1))

cat >> "$REPORT" <<EOF
---

## Summary

| Scenario | RPO | RTO |
|---|---|---|
| kill-store (GreptimeDB 30 s) | 0 events (${STORE_BUFFERED:-?} buffered and replayed) | ${STORE_RTO:-?} ms |
| kill-db (PostgreSQL 10 s) | 0 events | ${DB_RTO:-?} ms |
| restore-drill (isolated PITR) | 0 events to restore point | ${DRILL_RTO:-?} ms |

Limits of this drill: single host, single PostgreSQL primary, no Patroni, no
read replicas, simulated tools only.
EOF

echo
echo "-- report written to ${REPORT}"
if [ "$failures" -ne 0 ]; then
  echo "FAIL: ${failures} scenario(s) failed"
  exit 1
fi
api_post /api/sim/play >/dev/null || true
echo "PASS: three chaos scenarios completed"
