#!/usr/bin/env bash
# Truncate the derived time series store and rebuild it from PostgreSQL's
# lot_event. Counts and the SPC average must come back identical.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"
start_log replay

echo "-- pausing the line and draining the outbox"
api_post /api/sim/pause >/dev/null
wait_snapshot 'd["db"]["outbox_pending"] == 0' 90 || { echo "FAIL: outbox did not drain"; exit 1; }

echo "-- before"
before=$(api_get /api/stats)
echo "$before" | python3 -c '
import json,sys
d=json.load(sys.stdin)
print("   " + " ".join(f"{k}={v}" for k,v in sorted(d["greptime"].items())))
print("   spc_avg={:.9f} events={}".format(d["spc_average"], d["db"]["events"]))'

echo "-- truncating GreptimeDB and replaying every event"
replay=$(api_post /admin/replay '{"truncate":true}')
echo "$replay"

echo "-- after"
after=$(api_get /api/stats)
echo "$after" | python3 -c '
import json,sys
d=json.load(sys.stdin)
print("   " + " ".join(f"{k}={v}" for k,v in sorted(d["greptime"].items())))
print("   spc_avg={:.9f} events={}".format(d["spc_average"], d["db"]["events"]))'

python3 - "$before" "$after" <<'PY' | tee "$ARTIFACTS/replay-comparison.txt"
import json
import sys

before = json.loads(sys.argv[1])
after = json.loads(sys.argv[2])

failures = []
before_counts = {k: int(v) for k, v in before["greptime"].items()}
after_counts = {k: int(v) for k, v in after["greptime"].items()}
for table in sorted(set(before_counts) | set(after_counts)):
    a = before_counts.get(table, 0)
    b = after_counts.get(table, 0)
    status = "same" if a == b else "DIFF"
    print(f"   {table:<22} before={a:<6} after={b:<6} {status}")
    if a != b:
        failures.append(f"{table}: {a} -> {b}")

delta = abs(before["spc_average"] - after["spc_average"])
print(f"   spc_average            before={before['spc_average']:.9f} after={after['spc_average']:.9f} delta={delta:.3e}")
if delta > 1e-9:
    failures.append(f"spc average changed by {delta}")

if failures:
    print("FAIL: " + "; ".join(failures))
    sys.exit(1)
print("PASS: the time series store was rebuilt exactly from lot_event")
PY

api_post /api/sim/play >/dev/null
echo "-- line resumed"
