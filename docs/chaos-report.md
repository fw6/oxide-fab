# oxide-fab chaos report

Generated: 20261116T165455Z (UTC)

> **Restart-based recovery, not automatic failover.** Every RTO below includes
> the container restart time. RPO is measured from the event log: an event that
> PostgreSQL committed is never lost, and the outbox re-delivers it.

## 1. kill-store — GreptimeDB stopped for 30 s

- Events before the outage: **48**
- Events committed to PostgreSQL during the outage: **60** (the outbox absorbed all of them)
- Outbox backlog observed during the outage: **48** events
- **RPO: 0 events lost** — every event stayed in PostgreSQL's outbox
- **RTO: 48471 ms** — from `docker compose stop greptimedb` until the outbox drained
- After catch-up: events=108, published=108, pending=0, Greptime rows=126
- Lot state preserved: completed lots before=0, after=6
- Evidence: `/Users/fengwei/Projects/private/oxide-fab/artifacts/chaos-20261116T165455Z.log`
- Conclusion: the collector never lost a point; delivery resumed automatically. This is buffering + catch-up, not a failover.

## 2. kill-db — PostgreSQL stopped for 10 s

- `GET /api/snapshot` during the outage: **HTTP 503, 503, 503** (the API refuses writes without a database)
- Events before: **48**, after: **94** (the batch that was in flight committed after recovery)
- **RPO: 0 events lost**
- **RTO: 18941 ms** — including the PostgreSQL restart and WAL recovery
- After recovery: published=94, outbox pending=0
- Evidence: `/Users/fengwei/Projects/private/oxide-fab/artifacts/chaos-20261116T165455Z.log`
- Conclusion: no dirty writes while the database was gone; the buffered batch committed once it returned. Restart-based recovery, not automatic failover.

## 3. secs-reconnect — HSMS link dropped for 5 s

- Equipment spool peak while the link was down: **37 GEM events**
- Replayed from the drop until the spool emptied: **37 events** (peak 37; S6F11 sent: 909, S5F1 sent: 0)
- **RPO: 0 GEM events** — the equipment spool held every report (spooled total 909, replayed total 909, unsent 0)
- **RTO: 5323 ms** — from the drop until the spool drained and the link was up again
- Link drops: 6 -> 7
- Evidence: `artifacts/secs-reconnect.json`, `/Users/fengwei/Projects/private/oxide-fab/artifacts/chaos-20261116T165455Z.log`
- Conclusion: the tool spooled S6F11/S5F1 reports and replayed them on reconnect, the same buffering idea as the outbox one layer down.

## 4. restore-drill — isolated point-in-time restore

- Target time: **2026-11-16 16:56:20.969048+00**
- Restored rows: lots=**6**, events=**106**, Greptime rows=**24**
- pg_is_in_recovery()=false; row counts and `max(created_at)` matched the source
- **RPO: 0 events** to the restore point
- **RTO: 90769 ms** (down → pgBackRest restore → up → verify → import → verify)
- Evidence: `artifacts/restore-latest.json`, `artifacts/chaos-restore-20261116T165455Z.log`
- Conclusion: an isolated project restored the cluster without touching the running one.

---

## Summary

| Scenario | RPO | RTO |
|---|---|---|
| kill-store (GreptimeDB 30 s) | 0 events (60 buffered and replayed) | 48471 ms |
| kill-db (PostgreSQL 10 s) | 0 events | 18941 ms |
| secs-reconnect (HSMS 5 s) | 0 GEM events (37 spooled, 37 replayed) | 5323 ms |
| restore-drill (isolated PITR) | 0 events to restore point | 90769 ms |

Limits of this drill: single host, single PostgreSQL primary, no Patroni, no
read replicas, simulated tools only.
