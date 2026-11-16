# ADR 003 — Idempotent delivery into GreptimeDB

- **Status:** accepted
- **Date:** 2026-11-16
- **Decision:** every derived table's primary key contains `event_id`, the
  payload of an outbox row is immutable, and the delivery contract is
  *at-least-once with a deduplicating key* — never a distributed transaction.

## The question

The outbox gives at-least-once delivery. What stops a redelivered event from
adding a second point?

## Measurement (done before the implementation was fixed)

GreptimeDB `v1.3.1` (image `greptime/greptimedb:v1.3.1`). The tables are plain
`mito` tables: `append_mode` defaults to `'false'`, and the documented rule is
that duplicate rows are merged by **primary key + time index**
(`reference/sql/create.md`: *"Default is 'false', which removes duplicate rows
by primary keys and timestamps according to the `merge_mode`"*).

Probe on the live instance, through `scripts/greptime-sql.sh`:

```sql
CREATE TABLE adr003_probe (
  ts TIMESTAMP TIME INDEX,
  event_id STRING,
  param STRING,
  `value` DOUBLE,
  PRIMARY KEY (event_id, param)
);
INSERT INTO adr003_probe VALUES ('2026-11-16 12:00:00.000'::TIMESTAMP_MS,
  '11111111-1111-4111-8111-111111111111', 'cd', 100.5);
INSERT INTO adr003_probe VALUES ('2026-11-16 12:00:00.000'::TIMESTAMP_MS,
  '11111111-1111-4111-8111-111111111111', 'cd', 100.5);
SELECT count(*) FROM adr003_probe;   -- 1
```

Then the same `event_id` at a **different** timestamp:

```sql
INSERT INTO adr003_probe VALUES ('2026-11-16 12:00:10.000'::TIMESTAMP_MS,
  '11111111-1111-4111-8111-111111111111', 'cd', 100.7);
SELECT count(*) FROM adr003_probe;   -- 2
```

Result: identical rows (same primary key **and** same timestamp) collapse to
one; the same key at another timestamp does not. The time index is part of the
storage key, so deduplication only works if the retry carries the *same*
timestamp.

## How the contract is enforced

1. `oxide_core::metrics::derive(event, route)` is pure: one event always yields
   the same rows, including `ts = occurred_at_ms` and `event_id`.
2. The outbox payload is that derived vector, written inside the same
   PostgreSQL transaction as `lot_event`. A redelivery reads the stored payload;
   it never re-derives with a new clock.
3. Every table's `PRIMARY KEY` contains `event_id`:

   | Table | Primary key |
   |---|---|
   | `equipment_telemetry` | `(event_id, param)` |
   | `gem_event` | `(event_id)` |
   | `spc_sample` | `(event_id)` |
   | `e116_perf` | `(event_id)` |

4. The publisher marks `outbox.published_at` and inserts `published_event` in
   one transaction **after** the store accepted the rows. A crash between the
   two means a redelivery, which criterion 3 absorbs.

## Acceptance evidence

`./scripts/idempotency.sh` asks the server to redeliver a real `track_out`
event twice by event id (`POST /admin/redeliver/{event_id}`), which re-derives
nothing and re-inserts the stored rows:

```text
redelivering event 00000000-0135-48fc-8000-00000000006a
attempt: rows=4 delta=0 idempotent=True before={'e116_perf': 24, ...} after={'e116_perf': 24, ...}
attempt: rows=4 delta=0 idempotent=True before={'e116_perf': 24, ...} after={'e116_perf': 24, ...}
PASS: the same event_id delivered twice added no point
```

## Explicit non-goal

There is **no distributed transaction and no 2PC between PostgreSQL and
GreptimeDB**. PostgreSQL is the single source of truth; GreptimeDB is derived
and can be rebuilt from `lot_event` at any time (`scripts/replay.sh` proves it
by truncating the tables and replaying every event, then comparing counts and
the SPC average).

## Consequence / limit

Because the timestamp is part of the deduplication key, a payload that changed
its timestamp would create a second point. That is exactly why the outbox
payload is treated as immutable data and replay never touches `occurred_at`.
