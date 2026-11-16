# Changelog

All notable changes to `oxide-fab` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

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
