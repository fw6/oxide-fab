# Architecture

## One line

A simulated 300 mm line produces lots and wafers; every state change is an
event in PostgreSQL; a transactional outbox feeds GreptimeDB, which serves the
SPC chart; a Rust nsh dashboard shows the floor and proves the consistency
contract while chaos runs.

## Layers and dependency direction

```
               ┌──────────────────────────────┐
               │  bins/oxide-server  (oxide)  │   wiring only
               └───────┬──────────────┬───────┘
                       │              │
        ┌──────────────▼───┐   ┌──────▼──────────┐   ┌─────────────────┐
        │   oxide-api      │   │  oxide-secs     │   │  oxide-ui       │
        │ axum routes/DTOs │   │ HSMS-SS pair    │   │ nsh dashboard   │
        └───────┬──────────┘   └──────┬──────────┘   └────────┬────────┘
                │                     │                       │ HTTP
        ┌───────▼──────────┐   ┌──────▼──────────┐            │
        │   oxide-store    │   │  oxide-core     │◄───────────┘ (wire types)
        │ PG + outbox +    │   │ pure domain     │
        │ Greptime writer  │   │ (no I/O)        │
        └───────┬──────────┘   └──────┬──────────┘
                │                     │
        ┌───────▼──────────┐   ┌──────▼──────────┐
        │ PostgreSQL       │   │  oxide-sim      │
        │ (truth)          │   │ BinaryHeap DEE  │
        └──────────────────┘   └─────────────────┘
```

Rules enforced by the manifests:

- `oxide-core` depends on **serde/serde_json/sha2/uuid only**. No axum, no
  sqlx, no tokio, no network. `cargo test -p oxide-core` needs no database.
- `oxide-sim` depends on `oxide-core` and serde. It owns time and randomness;
  it does not know about HTTP, SQL or GreptimeDB.
- `oxide-store` is the only crate that knows SQL. It exposes the event store,
  the outbox projector and the GreptimeDB client.
- `oxide-api` depends on `oxide-sim` and `oxide-store`; it never talks to
  GreptimeDB with its own SQL string (it calls `oxide-store`).
- `oxide-secs` depends on `oxide-core` (recipe checksums) and `secs4rs`; it has
  no database knowledge.
- `oxide-ui` is a separate workspace (nsh builds runners from it) and speaks
  only HTTP. It duplicates the wire DTOs on purpose: a UI that shared Rust
  types with the server would hide a contract drift instead of exposing it at
  the API boundary.

## The write path (the important one)

```
sim.tick() ──► DomainEvent[] ──► PgStore::append_events(events)
                                   │
                     ┌─────────────┴───────────────────────────┐
                     │ BEGIN                                   │
                     │  INSERT lot_event (event_id UNIQUE)     │
                     │  project lot / wafer / equipment        │
                     │  INSERT outbox (payload = derived rows) │
                     │ COMMIT                                  │
                     └─────────────┬───────────────────────────┘
                                   │
             Publisher (tokio task): claim FOR UPDATE SKIP LOCKED
                                   │  POST /v1/sql (INSERT …)
                                   ▼
                        GreptimeDB (dedupe by event_id + ts)
                                   │
                     BEGIN mark outbox.published_at + published_event COMMIT
```

Failure behaviour:

| Failure | What happens | RPO |
|---|---|---|
| The append transaction fails | The batch stays in memory, the engine is paused, the batch is retried every 400 ms | 0 while the process lives |
| GreptimeDB is down | Rows stay unpublished in `outbox`; the publisher retries | 0 (PG holds them) |
| The process dies | Restart resumes from `snapshot` + `lot_event` after `last_seq` | 0 for committed events |
| GreptimeDB loses its volume | `scripts/replay.sh` rebuilds every table from `lot_event` | 0 |

## Read paths

- `GET /api/snapshot` — the engine's view (queues, tools, lots), DB counts,
  outbox stats, SPC buckets, SECS status, backup/restore reports.
- `GET /api/events?since=<seq>` — the incremental feed the dashboard folds into
  its own state. Sequence numbers come from `lot_event.seq`.
- `GET /api/lots/{id}` — lot + wafers + per-wafer step records, folded from the
  event log on read (genealogy is never stored twice).

## State machine

`oxide-core::state_machine::LotAggregate` is the only place that mutates a lot.
Every method returns `Result<Transition, TransitionError>`; a transition
returns the events it produced. Illegal moves (`track_out` before `track_in`,
rework forward, merge across steps, a recipe that was not checksum-verified)
are refused before anything reaches the log. A repeated `track_out` for wafers
that already finished the step is recognised by the per-step completed set and
turns into a `track_out_duplicate` audit event instead of advancing material.

## Simulator

`oxide-sim` keeps a `BinaryHeap<Reverse<Scheduled>>` (release, step complete,
repair), one `VecDeque` of lots per step and a bucket of tools per equipment
kind. Dispatch is hand-written: highest priority first, FIFO within a
priority, a qualified idle tool, and a verified recipe. `SimState` holds
everything needed to resume (clock, RNG, lots, queues, tool busy-until), so a
restart rebuilds the heap from state instead of serialising a heap.

## Time series model

| Table | Primary key | Written when |
|---|---|---|
| `equipment_telemetry` | `(event_id, param)` | a step completes |
| `gem_event` | `(event_id)` | S6F11/S5F1/S7 events are derived from lot events |
| `spc_sample` | `(event_id)` | a metrology step completes |
| `e116_perf` | `(event_id)` | processing and down durations |

`spc_downsample` is a continuous aggregation (`CREATE FLOW … SINK TO …`) over
10-second windows; the dashboard reads it, and falls back to raw samples while
the first bucket is filling.

## Deployment

One compose file: `postgres` (pgBackRest, WAL archiving to MinIO), `minio`,
`minio-init`, `greptimedb` (S3 backend) and `oxide`. A separate
`docker-compose.restore.yml` project restores into isolated volumes and its own
S3 prefix (`/oxide-restore-verify`).
