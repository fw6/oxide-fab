# ADR 004 — Object store: RustFS instead of MinIO

- **Status:** accepted (forced substitution, documented)
- **Date:** 2026-11-16
- **Decision:** run **RustFS** (`rustfs/rustfs:1.0.6`) as the S3-compatible
  object store, keeping the bucket names `oxide-data` and `oxide-backup` and
  the S3 configuration of every client (GreptimeDB, pgBackRest, the export
  tooling).

## Why this ADR exists

The brief specifies MinIO. As of **2026-11-16** MinIO no longer publishes
anything the CI or a fresh machine can pull:

| Check | Result |
|---|---|
| `docker pull minio/minio:latest` (Docker Hub, direct) | `pull access denied … repository does not exist or may require 'docker login'` |
| Anonymous bearer token + `GET /v2/minio/minio/manifests/latest` | **401** |
| Same, by digest `sha256:14cea…936e` | **401** |
| `quay.io/minio/minio` | 401 |
| `ghcr.io/minio/minio` | denied |
| `https://dl.min.io/server/minio/release/linux-amd64/archive/minio.RELEASE.2025-09-07T16-13-09Z` | **410 Gone** |
| `https://dl.min.io/server/minio/release/linux-amd64/minio` | **410 Gone** |

The earlier local runs used an image already present in this machine's cache;
GitHub's runner has no cache, which is how CI exposed the removal.

## Chosen alternative

[RustFS](https://github.com/rustfs/rustfs) — Apache-2.0, written in Rust,
S3-compatible, and explicitly built for migration/coexistence with MinIO. It
serves the same API on port 9000, accepts path-style requests, and needs only
`RUSTFS_ACCESS_KEY` / `RUSTFS_SECRET_KEY`.

What stays identical:

- Bucket names `oxide-data` (GreptimeDB data) and `oxide-backup` (pgBackRest
  repository, logical dumps, Greptime Export/Import V2 snapshots).
- GreptimeDB standalone's S3 configuration
  (`GREPTIMEDB_STANDALONE__STORAGE__*`).
- pgBackRest's `repo1-s3-*` configuration.
- The scripts' S3 URLs (`s3://oxide-backup/...`).
- The separation of the backup repository from the PostgreSQL volume.

What changes: one compose service (`rustfs` instead of `minio`) and a bucket
initialiser that uses `aws s3 mb` (the `amazon/aws-cli` image) because the
MinIO `mc` client is no longer pullable either.

## Verified after the swap

All of this ran on the RustFS stack:

- `./scripts/smoke.sh` — six lots, four steps, complete genealogy, 108 events
  delivered, `equipment_telemetry=24 gem_event=72 spc_sample=6 e116_perf=24`.
- `./scripts/backup.sh` — pgBackRest full backup and WAL archive accepted by
  RustFS, logical dumps uploaded, Greptime snapshot created and verified
  (`s3://oxide-backup/greptime/…`).
- `./scripts/restore.sh` — `pgbackrest restore --type=time --target-action=promote`
  from the RustFS repository into the isolated project, then a Greptime
  `import-v2` from the RustFS snapshot: lots=6, events=108,
  `max(created_at)` matched, `equipment_telemetry=24`.
- `./scripts/replay.sh`, `./scripts/idempotency.sh`, `./scripts/chaos.sh`.

## If MinIO becomes available again

Replace the `rustfs` service with `minio/minio:<release>` and the
`bucket-init` command with `mc mb`, then point the four environment blocks back
at `minio:9000`. Nothing else changes, because every client speaks S3 and the
bucket names are unchanged.
