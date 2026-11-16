#!/usr/bin/env bash
# Deliver one event's rows a second time and prove the time series store does
# not grow: the event_id is part of every table's primary key.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"
start_log idempotency

echo "-- pausing the line and draining the outbox"
api_post /api/sim/pause >/dev/null
wait_snapshot 'd["db"]["outbox_pending"] == 0' 90 || { echo "FAIL: outbox did not drain"; exit 1; }

last_seq=$(api_get /api/stats | json last_seq)
since=$(( last_seq > 400 ? last_seq - 400 : 0 ))
event_id=$(api_get "/api/events?since=${since}&limit=400" | python3 -c '
import json, sys
d = json.load(sys.stdin)
preferred = [e for e in d["events"] if e["event"]["kind"] == "track_out"]
fallback = [e for e in d["events"] if e["event"]["kind"] in ("track_in", "equipment_down")]
chosen = (preferred or fallback or d["events"])
if not chosen:
    raise SystemExit("no events to redeliver")
print(chosen[-1]["event"]["event_id"])')
echo "   redelivering event ${event_id}"

for attempt in 1 2; do
  result=$(api_post "/admin/redeliver/${event_id}")
  echo "$result" | python3 -c '
import json, sys
d = json.load(sys.stdin)
print("   attempt: rows={} delta={} idempotent={} before={} after={}".format(
    d["rows_redelivered"], d["delta"], d["idempotent"], d["before"], d["after"]))
if not d["idempotent"] or d["delta"] != 0:
    raise SystemExit("FAIL: the store accepted a duplicate event")
'
done

echo "PASS: the same event_id delivered twice added no point"
api_post /api/sim/play >/dev/null
