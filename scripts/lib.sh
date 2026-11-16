#!/usr/bin/env bash
# Shared helpers for the oxide-fab scripts. Source this file, do not run it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
fi

API="${API:-http://127.0.0.1:8080}"
ARTIFACTS="${ARTIFACTS:-$ROOT/artifacts}"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$ARTIFACTS"

# Every script logs to artifacts/<name>-<timestamp>.log and echoes to stdout.
start_log() {
  local name="$1"
  LOG="$ARTIFACTS/${name}-${TS}.log"
  exec > >(tee "$LOG") 2>&1
  echo "== ${name} @ ${TS} =="
}

now_ms() { python3 -c 'import time; print(int(time.time() * 1000))'; }
now_utc() { date -u +%Y-%m-%dT%H:%M:%SZ; }

api_get() { curl -fsS "${API}$1"; }

api_post() {
  local path="$1"
  local body="${2:-}"
  if [ -n "$body" ]; then
    curl -fsS -X POST -H 'Content-Type: application/json' -d "$body" "${API}${path}"
  else
    curl -fsS -X POST "${API}${path}"
  fi
}

http_code() { curl -sS -o /dev/null -w '%{http_code}' "${API}$1" 2>/dev/null || echo 000; }

json() {
  python3 -c '
import json, sys
value = json.load(sys.stdin)
path = sys.argv[1]
if path:
    for key in path.split("."):
        value = value[int(key)] if key.isdigit() else value[key]
print(value)' "$1"
}

wait_http() {
  local path="${1:-/healthz}"
  local timeout="${2:-60}"
  local start
  start=$(date +%s)
  while :; do
    if [ "$(http_code "$path")" = "200" ]; then
      return 0
    fi
    if [ $(( $(date +%s) - start )) -ge "$timeout" ]; then
      return 1
    fi
    sleep 1
  done
}

wait_snapshot() {
  # wait_snapshot "<python expression over d>" <timeout seconds>
  local expression="$1"
  local timeout="${2:-120}"
  local start
  start=$(date +%s)
  while :; do
    if api_get /api/snapshot | python3 -c "import json,sys; d=json.load(sys.stdin); sys.exit(0 if ($expression) else 1)"; then
      return 0
    fi
    if [ $(( $(date +%s) - start )) -ge "$timeout" ]; then
      return 1
    fi
    sleep 1
  done
}

stats_summary() {
  api_get /api/stats | python3 -c '
import json, sys
d = json.load(sys.stdin)
print("postgres   lots={lots} wafers={wafers} events={events} outbox_pending={outbox_pending} published={outbox_published}".format(**d["db"]))
print("greptime   " + " ".join(f"{k}={v}" for k, v in sorted(d["greptime"].items())))
print("spc_avg    {:.6f}".format(d["spc_average"]))
print("last_seq   {}".format(d["last_seq"]))'
}
