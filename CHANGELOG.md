# Changelog

All notable changes to `oxide-fab` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- **The write path no longer races itself.** Decisions and their appends now
  share `write_lock` for the simulator and every operator action, so
  `lot_event` order is the order the aggregate accepted; before, two writers
  could commit in the opposite order and a restart could undo a hold or
  re-dispatch a held lot. An operator append that fails puts the copied state
  back instead of leaving a phantom. While the simulator holds a batch it could
  not append, operator writes, `/api/snapshot` and what-if answer 503 rather
  than reading state that is ahead of the log.
- **A snapshot is now one critical section.** `SimState` carries the
  `lot_event.seq` it reflects, captured with the state under `write_lock`
  instead of a later `max_seq()` query, so a restart cannot skip a committed
  event. `append_events` also returns the sequence of an event that is already
  present, so retrying a committed-but-unacknowledged batch cannot replay it.
- **Reset and replay cannot race the publisher.** A `delivery_lock` is held
  across a delivery batch and across the truncates in `/admin/reset`,
  `/admin/replay` and redelivery, so a claimed batch cannot land in a truncated
  store.

### Changed

- **PostgreSQL 16 → 18** (`postgres:18-bookworm`, 18.7). PG18 keeps the cluster
  in `/var/lib/postgresql/18/docker` with the volume at `/var/lib/postgresql`,
  so `pg1-path` and both compose files moved with it; bookworm's pgBackRest 2.45
  cannot read a PG18 cluster, so it comes from PGDG (2.60.1). The migration
  route for an existing PG16 volume and the pgBackRest repository window are in
  [ADR 009](docs/decisions/009-postgres-18.md).
- The restore drill builds the isolated image and restores as the postgres user;
  without either, it tested a stale image or a root-owned `PGDATA`.

## [0.2.0] — 2026-11-18

### Added

- **Desktop 3D workbench**: a wgpu viewport in an nsh native component
  (`#[native_components(macos|linux|windows)]`, registered by the app's own
  crate), fed by `oxide-render`; Blender-exported GLB plus a manifest that names
  every node, part, picking box and explosion direction.
- `oxide-scene`: manifest contract, AMHS layout, orbit camera (ortho and
  perspective), CPU ray/AABB picking and the L0–L3 scene graph; no GPU, fully
  unit-tested.
- `oxide-render`: glTF loading with baked node transforms, mesh and line
  pipelines, offscreen frames and `oxide-render-evidence` (four levels, an
  orthographic frame and an adapter report; `--validate-only` for assets).
- Read-only API: `GET /api/analytics` (recipes, Q-time, alarms, KPI with
  formulas, plan vs actual), `GET /api/monitor`, `GET /api/events/{event_id}`
  (event → outbox → GreptimeDB rows) and `POST /api/what-if` (isolated replay
  of a snapshot copy).
- Dispatch decisions recorded by the simulator (`rule`, priority, queue
  position, qualified tools) and exposed in the snapshot; `DispatchRule` adds
  FIFO for the what-if.
- A five-domain workbench UI with a guided demo, light/dark themes, reduced
  motion, label density, JSON exports and a local-compose button; all writes
  are labelled 演示触发 and go through `oxide-api`.
- Three native packages (macOS `.app`, Linux directory, Windows directory and
  Inno Setup installer when available) and a three-OS CI matrix that packages,
  runs and screenshots each, plus software-rendering evidence (lavapipe, WARP).
- ADRs 005–008 and `docs/design/` (client features, four visual proposals).

### Changed

- `route_step` gains `qtime_ms` in the scenario (no migration); Q-time is
  computed from `track_out → track_in` gaps.
- Phase-1 acceptance item 2 is re-scoped: `nsh test` plus three desktop
  packages replace the web/TUI artefacts (ADR 005).
- `minimum system version`, window size, icon and app version come from
  `nsh.json`.

### Removed

- The web and TUI packages and their release jobs; the desktop application is
  the only UI deliverable.
- The TSX shell test: the app has no TSX entry, so `nsh test` now builds only
  the Rust test host (no QuickJS in the app or its tests).

### Changed (phase-1 review carry-over)

- The workspace enables the Pragmatic Rust Guidelines' recommended lint set
  (all clippy groups plus the rust lints) and CI enforces it with
  `-D warnings`; `docs/rust-guidelines-review.md` records the review, the
  opt-outs and the one remaining structural item (typed event payloads).
- `GET /api/snapshot` no longer repeats the floor: `steps`, `equipment` and
  `lots` live only under `sim`. The dashboard reads them from there.
- A lot the operator names that does not exist is now **404**, not 400; an
  illegal transition is still 400.

### Fixed

- An unknown `lot_event.kind` (or `lot`/`wafer` state) is an error instead of
  being silently read as `lot_created` / `created`, which would have been
  projected and replayed as truth.
- `POST /api/lots/{id}/merge` is all-or-nothing: an unknown source id no longer
  strands the lots that were already taken out of the simulator.
- A SECS message whose text the encoder refuses now reports an error instead of
  panicking the equipment thread.
- A GreptimeDB query failure is reported as such, not as "no samples".
- A lot whose life has ended (`completed`, `split`, `merged`) refuses every
  process action with `already completed`; before, a split lot could be
  reworked and a completed lot could take another track-in, silently reviving
  it. The whole lifecycle is now pinned by an exhaustive 8 × 11 matrix test.
- `complete` refuses a lot that still has material on a tool (`processing`), so
  the tool's track-out cannot arrive after the lot was declared finished.

## [0.1.2] — 2026-11-17

### Added

- Headless UI tests for the control bar: pressing 暂停 / 单步 / − / + and, on a
  paused line, 运行, is asserted through the stub network to POST
  `/api/sim/pause`, `/step`, `/speed/5`, `/speed/20` and `/play`.
- `docs/acceptance.md`: the twelve acceptance rows with the command, the result
  and the evidence path, including the explicit nsh artefact mapping (web is an
  `nsh package`; `tui` is `nsh build` + sources because no nsh release has a
  `package --target tui`).

## [0.1.1] — 2026-11-17

### Changed

- The container image is published for **linux/amd64 and linux/arm64**. The
  0.1.0 image was built for amd64 only, so `docker pull ghcr.io/fw6/oxide-fab`
  failed on Apple Silicon with `no matching manifest for linux/arm64/v8`; the
  Dockerfile already selected its musl target from `TARGETARCH`, the release
  job just never asked for a second platform.

## [0.1.0] — 2026-11-16

The first deliverable: a single-host, recoverable simulation of a four-step
semiconductor line.

### Added

- **Domain**: `oxide-core` with `enum` states, an explicit `transition()`
  machine for lots, and unit tests for split, merge, rework, hold/release,
  partial lots and a repeated track-out.
- **Simulation**: `oxide-sim`, a discrete-event engine on `BinaryHeap` with
  tool buckets, per-step queues, a hand-written dispatcher (priority, FIFO,
  the route step's `equipment_qualification` list and each tool's qualification
  matrix, verified recipe), failure injection and snapshot/resume.
- **Persistence**: PostgreSQL `lot_event` as the append-only truth, projections
  written in the same transaction, a transactional `outbox`, a
  `published_event` ledger, `snapshot` and `audit_log` (`migrations/`).
- **Delivery**: an outbox projector to GreptimeDB over the official HTTP SQL
  interface, at-least-once with `event_id` in every primary key.
- **Time series**: `equipment_telemetry`, `gem_event`, `spc_sample`,
  `e116_perf`, TTLs, and a `spc_downsample` continuous aggregation.
- **SECS/GEM**: `oxide-secs` on `secs4rs` (HSMS-SS over TCP) with S1F1, S6F11,
  S5F1 and S7F3/S7F4/S7F5/S7F6, plus equipment spooling and replay after a
  reconnect (feature `secs-gem`).
- **API**: axum routes for the dashboard snapshot, incremental event pull,
  per-wafer genealogy, SPC, simulation control, operator actions, SECS status
  and the replay, reset, redeliver and report admin endpoints.
- **UI**: `oxide-ui`, an nsh app in pure Rust mode for web and TUI.
- **Ops**: `scripts/smoke.sh`, `replay.sh`, `idempotency.sh`, `backup.sh`,
  `restore.sh`, `chaos.sh` — all run against the compose stack. The chaos
  report covers four scenarios: GreptimeDB down, PostgreSQL down, an HSMS link
  drop with equipment spool replay, and an isolated point-in-time restore.
- **Backup**: pgBackRest full backups and WAL archiving to the S3-compatible
  object store, logical dumps, GreptimeDB Export/Import V2 snapshots, and a
  point-in-time restore drill in an isolated compose project.
- **Object store**: MinIO, built from the pinned upstream source in
  `Dockerfile.minio` because MinIO's public images and binaries are no longer
  distributable; bucket names `oxide-data` / `oxide-backup` are unchanged
  (ADR 004).
- **CI/CD**: `ci.yml` (fmt, clippy, tests, audit, `nsh test`, the web package,
  actionlint and a full integration run) and `release.yml` (three platforms,
  checksums, GHCR, nsh web package, release notes).

### Fixed

- The release dry run (run 43925767572) failed to build the web package because
  the job had no `wasm32-unknown-unknown` target and no `llvm-ar`, which
  `nsh-platform-web` needs to compile zstd for wasm. CI now installs both and
  builds the web package on every push, so the release cannot regress there
  unnoticed.

### Known limits

- Single host, single PostgreSQL primary; recovery is a restart, not failover.
- The tools and the SECS/GEM peer are simulated; nothing here is production CIM.
- The TUI artifact is produced by nsh's build rather than `nsh package`, which
  has no TUI target in 0.5.0 (see the release notes).
