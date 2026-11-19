# Architecture

## One line

A simulated 300 mm line produces lots and wafers; every state change is an
event in PostgreSQL; a transactional outbox feeds GreptimeDB, which serves the
SPC chart; a **desktop wgpu workbench** shows the floor in 3D, drills from the
factory to a single `event_id`, and proves the consistency contract while chaos
runs.

## Layers and dependency direction

```
               ┌──────────────────────────────┐
               │  bins/oxide-server  (oxide)  │   wiring only
               └───────┬──────────────┬───────┘
                       │              │
        ┌──────────────▼───┐   ┌──────▼──────────┐   ┌──────────────────────┐
        │   oxide-api      │   │  oxide-secs     │   │  oxide-ui            │
        │ axum routes/DTOs │   │ HSMS-SS pair    │   │ nsh desktop app      │
        └───────┬──────────┘   └──────┬──────────┘   │  ├── oxide-scene     │
                │                     │              │  ├── oxide-render    │
        ┌───────▼──────────┐   ┌──────▼──────────┐   │  └── native modules  │
        │   oxide-store    │   │  oxide-core     │◄──┴──────── HTTP ──────┘
        │ PG + outbox +    │   │ pure domain     │      (snapshot, events,
        │ Greptime writer  │   │ (no I/O)        │       analytics, lot, trace)
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
- `oxide-sim` depends on `oxide-core`, serde and thiserror (for `SimError`). It
  owns time and randomness; it does not know about HTTP, SQL or GreptimeDB.
- `oxide-store` is the only crate that knows SQL. It exposes the event store,
  the outbox projector and the GreptimeDB client.
- `oxide-api` depends on `oxide-sim` and `oxide-store`; it never talks to
  GreptimeDB with its own SQL string (it calls `oxide-store`).
- `oxide-secs` depends on `oxide-core` (recipe checksums) and `secs4rs`; it has
  no database knowledge.
- `oxide-scene` is pure: manifest, layout, camera, picking, the draw list. It
  depends on `oxide-core` (the defect taxonomy and lot/wafer vocabulary) and
  glam, never on wgpu or nsh, so every rule is unit-tested headlessly.
- `oxide-render` owns wgpu and the GLB; it consumes `oxide-scene`'s graph and
  returns a frame. It has no windowing code and no domain knowledge.
- `oxide-ui` is a separate workspace (nsh builds runners from it) and speaks
  only HTTP. Snapshot DTOs are mirrored on purpose: a UI that shared Rust types
  with the server would hide a contract drift instead of exposing it at the API
  boundary. The pure read-model structs (`Analysis`, `WhatIfComparison`,
  `DispatchDecision`) are the one exception: the derivation itself is the
  contract, it is shared by the API and the UI, and serde keeps the wire shape
  in one place.

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
- `GET /api/analytics` — recipes/checksum verdicts, Q-time limits and
  violations, dispatch-neutral alarms, KPI and the formula notes; derived from
  the whole log by `oxide-core::analytics` (ADR 007).
- `GET /api/monitor` — PostgreSQL, `GreptimeDB`, the object store, the outbox
  publisher, this API and the SECS host, each with a status and latency.
- `GET /api/events/{event_id}` — the event, its outbox row and its rows in the
  four derived tables: the L3 provenance chain.
- `POST /api/what-if` — an offline replay of a **cloned** `SimState` with a
  changed dispatch rule or an extra tool; the live instance is untouched.

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

Two phase-2 additions stay inside this rule: `DispatchRule` (`priority → FIFO`
or `FIFO`, a field of `Simulation`, not of the snapshot) and a capped
`DispatchDecision` record in `SimState` — the inputs of the last 64 loads, for
the Work-Next panel. What-if runs the same engine on a cloned state.

## The 3D workbench

```
oxide-ui (nsh, UI thread)
  ├─ signals: floor, selection, level, camera
  ├─ builds the SceneGraph every 50 ms (oxide-scene::build)
  ├─ submits { graph, camera, size } to the render thread
  └─ native component shows the newest frame

render thread (oxide-render)
  ├─ glTF meshes (world transforms baked at load)
  ├─ camera + object uniforms, mesh and line pipelines, depth
  ├─ offscreen Rgba8UnormSrgb + readback
  └─ FrameSlot { version, width, height, RGBA }

native component (per platform)
  ├─ macOS: NSImageView in a pass-through NSView
  ├─ Linux: GtkPicture + GdkMemoryTexture
  └─ Windows: Direct2D bitmap into the component's box
```

Picking uses the same `Camera::view_projection` the renderer draws with and the
manifest's AABBs; a hit is the same `Entity` the panels and the search use.
Levels L0–L3 are views of one graph, not separate pages. The native view lets
the pointer through, so nsh's own mouse/wheel handlers own the input. The
renderer and its evidence binary share one code path (ADR 006).

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

One compose file: `postgres` (PostgreSQL 18 with pgBackRest from PGDG, WAL
archiving to MinIO), `minio` (built from its pinned source), `minio-init`,
`greptimedb` (S3 backend) and `oxide`. A separate
`docker-compose.restore.yml` project restores into isolated volumes and its own
S3 prefix (`/oxide-restore-verify`).

PG18 keeps the cluster in `/var/lib/postgresql/18/docker` and the volume at
`/var/lib/postgresql`; `pg1-path` follows it. The upgrade and the migration
route for an existing PG16 volume (pg_upgrade or dump/restore, plus the
pgBackRest stanza and repository window) are in
[ADR 009](decisions/009-postgres-18.md).

## CI

`ci.yml` has six jobs. `lint-and-test` runs fmt, clippy, the workspace tests and
`cargo audit`; `ui` runs the nsh tests, the asset check and the UI workspace's
own fmt/clippy; `desktop` is a three-OS matrix that renders the evidence frames,
packages with nsh, launches the app and screenshots it; `integration` brings the
compose stack up and runs smoke, idempotency, replay, backup, restore and chaos;
`workflows` is actionlint. Only `lint-and-test` gates the heavy jobs, so the
matrix and the stack run beside the UI lint rather than behind it; a pull request
run is cancelled when a newer commit supersedes it.

Caches, in the order that matters:

- `crates/oxide-ui/.nsh`, the generated nsh runner workspace. It costs minutes
  to rebuild from nothing and seconds to reuse, and it must be restored *before*
  the Rust target: the other order gives generated sources newer mtimes than the
  cached artefacts, and cargo rebuilds anyway.
- `crates/oxide-ui/target` and the root `target`, through `rust-cache`
  (dependency artefacts only, which is what the desktop and UI builds reuse).
- `/var/cache/apt/archives` for the GTK/Vulkan packages; the runner's mirror has
  ranged from 0.4 to 10.4 minutes for the same install.

`release.yml` calls `ci.yml` before it publishes, so a tag cannot ship anything
the matrix has not verified.
