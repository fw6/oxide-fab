# ADR 009 — PostgreSQL 18, and how an existing 16 cluster moves

- **Status:** accepted
- **Date:** 2026-11-19
- **Decision:** the demo runs PostgreSQL **18** (`postgres:18-bookworm`, 18.7),
  with pgBackRest **2.60.1 from the PGDG repository** so WAL archiving and PITR
  keep working. The compose volume moves from `/var/lib/postgresql/data` to
  `/var/lib/postgresql` and `pg1-path` to `/var/lib/postgresql/18/docker`.

## Why

Dependabot's image bump (#12) could not be merged as-is; CI failed for three
independent reasons, all reproduced before this change:

1. **PGDATA moved.** PG18 keeps the cluster in `/var/lib/postgresql/18/docker`
   and declares `/var/lib/postgresql` as the volume. Mounting the old
   `/var/lib/postgresql/data` makes the server refuse to start ("there appears
   to be PostgreSQL data in: /var/lib/postgresql/data (unused mount/volume)").
2. **Bookworm's pgBackRest is 2.45** (2023, PG16 era) and does not support a
   PG18 cluster. PGDG ships 2.60.1.
3. **Four more path sites** encoded the old layout: `PGBACKREST_PG1_PATH` in
   both compose files and the volume mount in both, plus the `rm -rf` in
   `scripts/restore.sh`. They move together or archiving/restore break.

The PostgreSQL packages in the official 18 image already come from PGDG
(`postgresql-18 18.7-1.pgdg12+2` is installed in the base image), so adding the
PGDG key and list installs pgBackRest **without** a second PostgreSQL server:
`dpkg -l` after the build shows the same five postgresql packages plus
`pgbackrest 2.60.1-1.pgdg12+1`.

## Consequences

- `docker compose up -d --wait` on empty volumes initialises
  `/var/lib/postgresql/18/docker`; `pgbackrest version` reports 2.60.1 and the
  stanza's `archive_command` keeps pushing WAL to `oxide-backup`.
- The restore drill needs the isolated project to build the same image
  (`--build`), and it must restore **as the postgres user**: pgBackRest creates
  `$PGDATA` with `0700` owned by the invoking user, and the entrypoint
  re-execs itself as postgres before it touches the data directory. A
  root-owned `18/` fails with `mkdir: cannot create directory
  '/var/lib/postgresql/18': Permission denied`. Both were latent before the
  upgrade and are fixed in `scripts/restore.sh`.
- Only `plpgsql` is installed, so there is no extension to rebuild.

## Moving an existing 16 cluster

The old volume holds a PG16 cluster in `pgdata:/var/lib/postgresql/data`. PG18
cannot open it. Two routes, both requiring writers to be stopped:

1. **pg_upgrade (fast, keeps the data files).** Run a one-off container built
   from a PGDG image that has both servers (`postgresql-16` and
   `postgresql-18`), mount the old directory and a new empty target, and run
   `pg_upgrade --check`, then `pg_upgrade`. Afterwards run
   `pgbackrest --stanza=oxide stanza-upgrade` so the stanza records the new
   version.
2. **dump/restore (simple, most portable).** `pg_dump -Fc` while the line is
   paused, initialise an empty PG18 cluster, `pg_restore` into it, then let the
   application's migrations confirm the schema. This is the route the demo uses
   when its volume is rebuilt; `scripts/smoke.sh` plus
   `scripts/idempotency.sh` prove the restored data behaves.

The pgBackRest repository is **not** backward-compatible across the version
boundary: restoring a PG16 backup set needs PG16 binaries, and PG18 restores
need a backup taken after the upgrade (or after `stanza-upgrade`). Keep the old
repository until its retention window ends; do not initialise a new repository
on top of the old prefix.

## Verified on the upgraded stack (2026-11-19)

| Check | Result |
|---|---|
| `docker compose up -d --wait` on fresh volumes | healthy; `PostgreSQL 18.7`, `data_directory=/var/lib/postgresql/18/docker`, `pgBackRest 2.60.1` |
| `scripts/smoke.sh` | lots=6 wafers=60 events=108 published=108 pending=0 |
| `scripts/idempotency.sh` | redelivery delta=0 |
| `scripts/replay.sh` | four tables rebuilt, SPC average delta 0 |
| `scripts/backup.sh` | stanza created; WAL `archived/failed/last` = `2/0/000000010000000000000004`; full backup `20261119-030811F`; logical dumps + Greptime snapshot |
| `pg_stat_archiver` after `pg_switch_wal()` | `archived_count` 15 → 16, `failed_count=0`, `last_archived_wal` advances |
| `scripts/restore.sh` (PITR, isolated project) | lots=6 events=108, Greptime 4 tables/24 rows, 70.6 s |
| `scripts/chaos.sh` | 4/4 PASS (RPO 0) |

Related: ADR 003 keeps `lot_event` the only truth, which is why the restore
drill is the acceptance gate for this change rather than "the container starts".
