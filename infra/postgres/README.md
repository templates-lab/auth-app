# Postgres

The application database: a persistent Postgres service with a healthcheck that
gates the API, a least-privilege application role, and scheduled logical backups
with a tested restore path.

This directory owns the database and its backups. It composes with the Traefik
edge (`infra/traefik/`) into the full stack; the API's dependency on a healthy
database is a thin override kept here (`api.override.yml`) so neither the edge
file nor this one has to know about the other's internals.

```
infra/postgres/
├── docker-compose.yml     # postgres service (+ optional backup sidecar), volumes, network
├── api.override.yml       # adds the API's depends_on/DATABASE_URL/backend network
├── initdb/
│   └── 10-app-role.sh     # first-init: create the least-privilege app role
├── scripts/
│   ├── pg-backup.sh       # one-shot pg_dump (custom format) + retention prune
│   ├── pg-restore.sh      # restore from a dump (clean, single transaction)
│   └── backup-scheduler.sh# sidecar entrypoint: back up now, then every interval
└── README.md
```

## Configuration

All settings come from the environment (put them in the repo `.env`). Two
secrets are **required** — the stack refuses to start without them:

| Variable                      | Required | Default       | Purpose                                                                  |
| ----------------------------- | -------- | ------------- | ------------------------------------------------------------------------ |
| `POSTGRES_SUPERUSER_PASSWORD` | yes      | —             | Bootstrap superuser password. Admin/backups only; the app never uses it. |
| `APP_DB_PASSWORD`             | yes      | —             | Password for the least-privilege application role.                       |
| `POSTGRES_SUPERUSER`          | no       | `postgres`    | Bootstrap superuser name.                                                |
| `POSTGRES_DB`                 | no       | `authapp`     | Application database name.                                               |
| `APP_DB_USER`                 | no       | `authapp_app` | Least-privilege application role name.                                   |
| `BACKUP_INTERVAL_SECONDS`     | no       | `86400`       | Seconds between scheduled backups (default: daily).                      |
| `BACKUP_RETENTION`            | no       | `7`           | Number of recent archives to keep.                                       |

The API reads `DATABASE_URL` (assembled in `api.override.yml`) plus the pool
tuning knobs the store crate understands: `DATABASE_MAX_CONNECTIONS`,
`DATABASE_MIN_CONNECTIONS`, `DATABASE_ACQUIRE_TIMEOUT_SECS`.

> These variables belong in the project `.env.example` (owned by the deployment
> docs work item). They are documented here so that item can pick them up.

## Persistence

The data directory lives in the named volume `pgdata`
(`/var/lib/postgresql/data`). It survives `docker compose down` and container
recreation; it is removed only by an explicit `docker compose down -v`. Backup
archives live in a second named volume, `pgbackups`.

## Healthcheck-gated startup

The `postgres` service declares a `pg_isready` healthcheck (10s interval, 5
retries, 30s start period to cover first-time `initdb`). The API declares
`depends_on: postgres: condition: service_healthy`, so Compose does not start the
API container until the database reports healthy. The app then connects its pool
eagerly at startup and its `/health` endpoint reflects live database
connectivity, so a database outage surfaces as an unhealthy API rather than
silent failures.

## Least-privilege application role

The container's `POSTGRES_USER` is the bootstrap **superuser**, used only for
administration and backups. On first initialisation, `initdb/10-app-role.sh`
creates a separate application role (`APP_DB_USER`) that is **not** a superuser
and **cannot create roles or databases** (`NOSUPERUSER NOCREATEDB NOCREATEROLE`).

That role is given ownership of the `public` schema of its own database, so the
app's migrations (which `CREATE` tables) succeed while its authority stays
confined to this one database. The trusted `pgcrypto` extension is pre-installed
by the superuser, so the app never needs extension-creation rights. The API's
`DATABASE_URL` connects as this role — never as the superuser.

## Backups and restore

Backups are logical dumps in Postgres custom format (`pg_dump --format=custom`),
which `pg_restore` can restore selectively and compressed. The `postgres-backup`
sidecar (Compose profile `backup`) runs `pg-backup.sh` on an interval and prunes
to `BACKUP_RETENTION` archives in the `pgbackups` volume. `wal-g` (continuous
archiving / PITR) is the natural upgrade path for production but is intentionally
out of scope for this template.

Enable scheduled backups:

```bash
docker compose \
  -f infra/postgres/docker-compose.yml \
  --profile backup up -d
```

Take a backup on demand:

```bash
docker compose -f infra/postgres/docker-compose.yml --profile backup \
  run --rm --entrypoint /scripts/pg-backup.sh postgres-backup
```

Restore from an archive (drops and recreates existing objects, in one
transaction):

```bash
docker compose -f infra/postgres/docker-compose.yml --profile backup \
  run --rm --entrypoint /scripts/pg-restore.sh postgres-backup /backups/authapp-<timestamp>.dump
```

## Running

Database only (standalone):

```bash
docker compose -f infra/postgres/docker-compose.yml up -d postgres
```

Full stack — edge + database + the API↔database wiring:

```bash
docker compose \
  -f infra/traefik/docker-compose.yml \
  -f infra/postgres/docker-compose.yml \
  -f infra/postgres/api.override.yml \
  up
```

## Verifying (backup/restore round-trip)

The commands below are the reproducible test for this component; they need a
Docker daemon (not available in every CI image). Expected results are noted
inline.

```bash
export POSTGRES_SUPERUSER_PASSWORD=devsuperpw APP_DB_PASSWORD=devapppw

# 1. Bring the database up and wait for healthy.
docker compose -f infra/postgres/docker-compose.yml up -d postgres
docker compose -f infra/postgres/docker-compose.yml ps
#   -> STATUS shows "healthy" once pg_isready passes.

# 2. The app role exists and is NOT a superuser.
docker compose -f infra/postgres/docker-compose.yml exec postgres \
  psql -U postgres -d authapp -c \
  "SELECT rolname, rolsuper, rolcreatedb, rolcreaterole FROM pg_roles WHERE rolname='authapp_app';"
#   -> authapp_app | f | f | f

# 3. Seed a row, take a backup, destroy the row, restore, confirm it is back.
docker compose -f infra/postgres/docker-compose.yml exec postgres \
  psql -U authapp_app -d authapp -c \
  "CREATE TABLE probe(id int primary key); INSERT INTO probe VALUES (1);"

docker compose -f infra/postgres/docker-compose.yml --profile backup \
  run --rm --entrypoint /scripts/pg-backup.sh postgres-backup
#   -> "backup: wrote ... /backups/authapp-<ts>.dump"

docker compose -f infra/postgres/docker-compose.yml exec postgres \
  psql -U authapp_app -d authapp -c "DROP TABLE probe;"

archive=$(docker compose -f infra/postgres/docker-compose.yml --profile backup \
  run --rm --entrypoint sh postgres-backup -c 'ls -1t /backups/*.dump | head -1')
docker compose -f infra/postgres/docker-compose.yml --profile backup \
  run --rm --entrypoint /scripts/pg-restore.sh postgres-backup "$archive"

docker compose -f infra/postgres/docker-compose.yml exec postgres \
  psql -U authapp_app -d authapp -c "SELECT count(*) FROM probe;"
#   -> 1   (the restore brought the dropped table and its row back)
```

### Behavioural test without Docker

The round-trip above needs a Docker daemon. For environments without one, a
hermetic harness stubs `pg_dump` / `pg_restore` on `PATH` and drives the real
`pg-backup.sh` and `pg-restore.sh` end-to-end, asserting the behaviours that
matter — atomic archive rename, retention pruning to the newest `N`, and the
restore contract (`--clean --if-exists --single-transaction --exit-on-error`,
plus the argument/error exit codes):

```bash
./infra/postgres/scripts/test-backup-restore.sh
#   -> result: 13 passed, 0 failed
```

This runs anywhere `bash` is available (no Postgres, no Docker), so the backup
and restore logic is now behaviourally covered — not merely syntax-checked
(`bash -n`). The Docker round-trip remains the full integration test to run
wherever a daemon is available, since only it exercises real `pg_dump`/
`pg_restore` against a live cluster.
