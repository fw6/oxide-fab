# ADR 004 — Object store: MinIO built from source

- **Status:** accepted
- **Date:** 2026-11-16
- **Decision:** run **MinIO** (`RELEASE.2025-10-15T17-29-55Z`), built from the
  official source tree in `Dockerfile.minio`, with the buckets `oxide-data`
  and `oxide-backup` and the S3 configuration of every client unchanged.

## Why the image is built instead of pulled

MinIO's prebuilt artefacts were withdrawn from public distribution; the
checks below were made on **2026-11-16**:

| Check | Result |
|---|---|
| `docker pull minio/minio:latest` (Docker Hub, direct) | `pull access denied … repository does not exist or may require 'docker login'` |
| Anonymous bearer token + `GET /v2/minio/minio/manifests/latest` | **401** |
| Same, by digest `sha256:14cea…936e` | **401** |
| `quay.io/minio/minio`, `ghcr.io/minio/minio` | 401 / denied |
| `https://dl.min.io/server/minio/release/linux-amd64/minio` | **410 Gone** |
| `https://dl.min.io/server/minio/release/linux-amd64/archive/minio.RELEASE.2025-09-07T16-13-09Z` | **410 Gone** |

The GitHub repository `minio/minio` is **archived** (last push 2026-04-24)
but still public, and its release tags, `go.mod` and `Dockerfile` are
reachable. That makes the official binary reproducible from source:

```text
go.mod at RELEASE.2025-10-15T17-29-55Z:  go 1.24.0 / toolchain go1.24.8
upstream Dockerfile:                     HTTP 200
```

## How it is built

`Dockerfile.minio`:

- `golang:1.24-alpine` clones the pinned tag and runs
  `CGO_ENABLED=0 go build -trimpath -ldflags "-s -w"`;
- the runtime is `scratch` (MinIO is a static Go binary), with CA
  certificates and a `/data` directory owned by uid 10001;
- no shell, so the compose healthcheck is not used: `minio-init` retries
  `aws s3 ls` until the server answers, then creates both buckets.

`docker compose up` therefore builds MinIO locally (about 3 minutes cold) and
runs it as a non-root user. The image is ~143 MB on arm64.

## What stays identical

- Bucket names `oxide-data` (GreptimeDB data) and `oxide-backup` (pgBackRest
  repository, logical dumps, Greptime Export/Import V2 snapshots);
- GreptimeDB standalone's `GREPTIMEDB_STANDALONE__STORAGE__*` S3 settings;
- pgBackRest's `repo1-s3-*` settings;
- every script (`s3://oxide-backup/...`), and the separation of the backup
  repository from the PostgreSQL volume;
- the one-shot `minio-init` service that creates the buckets.

## Licence

MinIO is **AGPL-3.0**. The image carries the unmodified upstream binary built
from the pinned tag; the source stays available at
`github.com/minio/minio/tree/RELEASE.2025-10-15T17-29-55Z`, and the running
container only serves S3 to this compose network. This is a demo deployment,
not a redistribution product.

## Verified after the switch

- `./scripts/smoke.sh` — 6 lots, 4 steps, complete genealogy, 108 events
  delivered, `equipment_telemetry=24 gem_event=72 spc_sample=6 e116_perf=24`.
- `./scripts/backup.sh` — pgBackRest full backup and WAL archive accepted by
  MinIO, logical dumps uploaded, Greptime snapshot created and verified.
- `./scripts/restore.sh` — PITR from the MinIO repository into the isolated
  project, then a Greptime import: lots/events/`max(created_at)` matched.
- `./scripts/replay.sh`, `./scripts/idempotency.sh`, `./scripts/chaos.sh`.

## If a prebuilt image becomes available again

Replace the `minio` service's `build:` with `image: minio/minio:<release>`,
keep the `minio-init` service, and nothing else changes.
