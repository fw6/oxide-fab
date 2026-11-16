# oxide-fab chaos report

Generated: 20261116T144212Z (UTC)

> **Restart-based recovery, not automatic failover.** Every RTO below includes
> the container restart time. RPO is measured from the event log: an event that
> PostgreSQL committed is never lost, and the outbox re-delivers it.

## 1. kill-store — GreptimeDB stopped for 30 s

- Events before the outage: **48**
- Events committed to PostgreSQL during the outage: **60** (the outbox absorbed all of them)
- Outbox backlog observed during the outage: **60** events
- **RPO: 0 events lost** — every event stayed in PostgreSQL's outbox
- **RTO: 47795 ms** — from `docker compose stop greptimedb` until the outbox drained
- After catch-up: events=108, published=108, pending=0, Greptime rows=126
- Lot state preserved: completed lots before=0, after=6
- Evidence: `/Users/fengwei/Projects/private/oxide-fab/artifacts/chaos-20261116T144212Z.log`
- Conclusion: the collector never lost a point; delivery resumed automatically. This is buffering + catch-up, not a failover.

## 2. kill-db — PostgreSQL stopped for 10 s

- `GET /api/snapshot` during the outage: **HTTP 503, 503, 503** (the API refuses writes without a database)
- Events before: **48**, after: **84** (the batch that was in flight committed after recovery)
- **RPO: 0 events lost**
- **RTO: 18095 ms** — including the PostgreSQL restart and WAL recovery
- After recovery: published=84, outbox pending=0
- Evidence: `/Users/fengwei/Projects/private/oxide-fab/artifacts/chaos-20261116T144212Z.log`
- Conclusion: no dirty writes while the database was gone; the buffered batch committed once it returned. Restart-based recovery, not automatic failover.

## 3. restore-drill — isolated point-in-time restore

- Target time: **2026-11-16 14:43:25.970024+00**
- Restored rows: lots=**6**, events=**86**, Greptime rows=**24**
- pg_is_in_recovery()=false; row counts and `max(created_at)` matched the source
- **RPO: 0 events** to the restore point
- **RTO: 91873 ms** (down → pgBackRest restore → up → verify → import → verify)
- Evidence: `artifacts/restore-latest.json`, `artifacts/chaos-restore-20261116T144212Z.log`
- Conclusion: an isolated project restored the cluster without touching the running one.

---

## Summary

| Scenario | RPO | RTO |
|---|---|---|
| kill-store (GreptimeDB 30 s) | 0 events (60 buffered and replayed) | 47795 ms |
| kill-db (PostgreSQL 10 s) | 0 events | 18095 ms |
| restore-drill (isolated PITR) | 0 events to restore point | 91873 ms |

Limits of this drill: single host, single PostgreSQL primary, no Patroni, no
read replicas, simulated tools only.
