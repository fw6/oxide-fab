# ADR 007 — Read-only analytics: recipes, Q-time, dispatch, KPI and what-if

- **Status:** accepted
- **Date:** 2026-11-17
- **Decision:** the workbench's numbers are derived **server-side** from
  `lot_event`, the scenario and the derived time series, exposed as one
  `GET /api/analytics` payload plus `GET /api/monitor`,
  `GET /api/events/{event_id}` and `POST /api/what-if`. No table is added; the
  migrations, the outbox contract and the ops scripts are unchanged.

## Why server-side

The UI polls `/api/events?since=` and folds increments for its live floor
view, but a KPI, a Q-time verdict or a dispatch decision needs the **whole**
log. Computing them in the browser-runtime app would duplicate the derivation
and make the same number disagree between panels. The derivations live in
`oxide-core::analytics` (pure, unit-tested) and `oxide-sim::whatif` (pure), and
the API passes the log through.

## Q-time: where the limit comes from

`RouteStep` gains `qtime_ms: Option<i64>`, declared per step in
`config/scenario.json` (steps 1–3: 60 s). It is **not** a new table or column:
the PostgreSQL `route_step` rows stay as they are (migrations are frozen), and
the API reads the same `Scenario` the engine runs. A DB-only consumer does not
see the limit; the workbench says so in the panel's source line
(`lot_event.occurred_at + route_step.qtime_ms`).

A violation is a pair of events in one lot:

```
elapsed = track_in(step n+1).occurred_at − track_out(step n).occurred_at
violation if elapsed > route_step(n).qtime_ms
```

The violation carries the lot, the wafers of the track-in, both steps, the
measured and allowed time, and the **`event_id` of the track-in**, which is
addressable at L3 (outbox row + `GreptimeDB` rows). Repeated pairs are counted
(`S1→S2 × 3`) as the cross-shift pattern.

## Recipes, dispatch, alarms

- **Recipes:** the seeded `recipe.version/checksum` is compared with every
  `track_out.recipe_checksum`. `verified_runs`, `mismatched_runs` and the status
  (`verified` / `mismatched` / `not_run`) are counts over the log; a mismatch is
  never silently accepted.
- **Dispatch:** the simulator records each decision when it loads a lot
  (`SimState.dispatch`, capped at 64): rule label, priority, queue position and
  length, the route step's qualified tools and the recipe. `/api/snapshot`
  carries them; the panel is the engine's own record, not a reconstruction.
- **Alarms:** `equipment_down` / `equipment_up` become alarm rows with their
  `event_id`; the panel's click opens the L3 trace.

## KPI formulas (shown in the UI next to each number)

| Metric | Derivation |
|---|---|
| 周期时间 | first event of the lot → `lot_completed.occurred_at`, mean/max |
| 产出 / 小时 | `lot_completed` count ÷ event-window hours |
| 利用率 | Σ `track_out.duration_ms` per tool ÷ event-window ms |
| 可用率 | 1 − Σ `equipment_up.down_ms` per tool ÷ window |
| 空窗 | window − busy − down (clamped at 0) per tool |
| FPY | 1 − `wafer_scrapped` ÷ distinct wafer ids seen |
| 缺陷分类 | keyword class (scratch/particle/resist/pattern/other) of `wafer_scrapped.reason` |
| WIP | lots with `lot_created` and no completed/split/merged |
| 计划 vs 实际 | route `duration_ms` per step vs mean `track_out.duration_ms` |
| 交付偏差 | lot cycle ÷ Σ route `duration_ms`; warning above +15 % |
| RPO / RTO | the backup/restore drill artefacts, read by `/api/backup` and `/api/restore`; never computed in the client |

`Analysis.notes` carries the same list, and the KPI card renders it; A-12's
"every number has a source" is enforced by construction, not by a caption.

## What-if: replay a snapshot copy, never the live instance

`POST /api/what-if { rule, add_tool_kind, horizon_ms }`:

1. Under the simulator lock: clone `Scenario` and `SimState` — the live
   instance is not mutated.
2. Build the variant scenario (one extra tool of the kind, with the recipes that
   kind runs, added to the route's qualification) or just change the dispatch
   rule (`priority → FIFO` vs `FIFO`).
3. Run the baseline and the variant from the **same snapshot**, each to
   completion or the horizon, on the blocking pool; events are discarded.
4. Compare completed lots, mean cycle time, throughput and WIP end; return the
   notes (same snapshot, formulas, current instance untouched).

The panel shows baseline → variant and the delta. The live line keeps running
throughout, which is the A-08 check.

## Monitor

`GET /api/monitor` checks, with a 1.5 s timeout each:

- PostgreSQL (`SELECT 1`),
- `GreptimeDB` (`SELECT 1`),
- the object store (`GET /minio/health/live`; `OXIDE_MINIO_URL`, default
  `http://minio:9000`, the compose service name),
- the outbox publisher (pending count and how long ago it last published;
  `down` when it has pending rows and has delivered nothing for 30 s),
- this API (uptime),
- the SECS host when the pair is enabled.

A down component is reported, never thrown: the workbench stays usable while a
backend is missing.

## Event trace

`GET /api/events/{event_id}` returns the `lot_event` row, its `outbox` row
(published time, attempts, last error) and the rows in the four derived tables
(`equipment_telemetry`, `gem_event`, `spc_sample`, `e116_perf`) whose
`event_id` matches. This is the L3 chain: the id in the footer, the copy button,
the outbox state and the time-series points.

## Consequences

- Two new store reads (`event_by_id`, `outbox_trace`, `outbox_health`) and one
  `GreptimeDB` query per event; all are indexed by `event_id`.
- `SimSnapshot` grows a `dispatch` array; old snapshots deserialize with
  `#[serde(default)]`.
- The API remains a read model: the only writes are the existing operator
  actions, plus the demo-triggered replay button that calls the existing
  `/admin/replay`.
