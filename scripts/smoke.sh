#!/usr/bin/env bash
# End-to-end smoke test: fresh state, release six lots, run all four steps,
# then prove every wafer has a complete genealogy and the outbox is drained.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
. "$SCRIPT_DIR/lib.sh"
start_log smoke

echo "-- waiting for the API"
wait_http /healthz 90 || { echo "FAIL: /healthz never returned 200"; exit 1; }

echo "-- resetting the fab (PostgreSQL, GreptimeDB and the engine)"
api_post /admin/reset >/dev/null

echo "-- starting the line"
api_post /api/sim/play >/dev/null
started_ms=$(now_ms)

echo "-- waiting for every lot to complete"
if ! wait_snapshot 'd["db"]["outbox_pending"] == 0 and all(l["state"] == "completed" for l in d["sim"]["lots"]) and len(d["sim"]["lots"]) > 0' 240; then
  echo "FAIL: the lots did not complete in time"
  api_get /api/snapshot | python3 -m json.tool | head -40
  exit 1
fi
elapsed_ms=$(( $(now_ms) - started_ms ))
echo "   all lots completed in ${elapsed_ms} ms of wall time"

echo "-- verifying per-wafer genealogy"
api_get /api/snapshot | python3 -c '
import json, sys
snapshot = json.load(sys.stdin)
for lot in snapshot["sim"]["lots"]:
    print("   {:<10} state={:<10} wafers={:<3} done={}".format(
        lot["id"], lot["state"], lot["wafer_count"], lot["done_wafers"]))
'

verification=$(python3 - <<'PY'
import json
import os
import urllib.request

api = os.environ.get("API", "http://127.0.0.1:18080")
config = json.load(urllib.request.urlopen(api + "/api/config"))
route = config["scenario"]["route"]["steps"]
expected_steps = {int(step["step_no"]): step for step in route}

snapshot = json.load(urllib.request.urlopen(api + "/api/snapshot"))
lots = snapshot["sim"]["lots"]
wafers = 0
problems = []
for lot in lots:
    detail = json.load(urllib.request.urlopen(api + "/api/lots/" + lot["id"]))
    if detail["lot"]["state"] != "completed":
        problems.append(f"{lot['id']} is {detail['lot']['state']}")
    by_wafer = {entry["wafer_id"]: entry for entry in detail["genealogy"]}
    for wafer in detail["wafers"]:
        wafers += 1
        entry = by_wafer.get(wafer["id"])
        if entry is None:
            problems.append(f"{wafer['id']} has no genealogy")
            continue
        if len(entry["steps"]) != len(expected_steps):
            problems.append(f"{wafer['id']} has {len(entry['steps'])} steps, expected {len(expected_steps)}")
            continue
        for record in entry["steps"]:
            step = expected_steps.get(record["step_no"])
            if step is None:
                problems.append(f"{wafer['id']} unknown step {record['step_no']}")
                continue
            if not record["equipment_id"]:
                problems.append(f"{wafer['id']} step {record['step_no']} has no tool")
            if record["recipe_id"] != step["recipe_id"]:
                problems.append(f"{wafer['id']} step {record['step_no']} recipe {record['recipe_id']} != {step['recipe_id']}")
            if record["track_out_ms"] <= record["track_in_ms"]:
                problems.append(f"{wafer['id']} step {record['step_no']} has no track-out")
            if record["duration_ms"] != step["duration_ms"]:
                problems.append(f"{wafer['id']} step {record['step_no']} duration {record['duration_ms']} != {step['duration_ms']}")
        finished = [record["step_no"] for record in entry["steps"]]
        if sorted(finished) != sorted(expected_steps):
            problems.append(f"{wafer['id']} finished steps {finished}")

print(json.dumps({
    "lots": len(lots),
    "wafers": wafers,
    "events": snapshot["db"]["events"],
    "published": snapshot["db"]["published_events"],
    "outbox_pending": snapshot["db"]["outbox_pending"],
    "problems": problems,
}, indent=2))
PY
)
echo "$verification"

if echo "$verification" | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(1 if d["problems"] or d["outbox_pending"] else 0)'; then
  echo "-- outbox and GreptimeDB"
  stats_summary
  echo "PASS: every lot completed and every wafer has a four-step genealogy"
else
  echo "FAIL: genealogy verification found problems"
  exit 1
fi
