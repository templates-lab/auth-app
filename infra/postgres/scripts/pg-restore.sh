#!/usr/bin/env bash
# Restore the application database from a custom-format pg_dump archive produced
# by pg-backup.sh. Existing objects are dropped first (--clean --if-exists) so
# the restore is a clean replace, not a merge. Connection settings come from the
# standard libpq environment variables (PGHOST, PGUSER, PGPASSWORD, PGDATABASE).
#
# Usage:
#   docker compose -f infra/postgres/docker-compose.yml --profile backup \
#     run --rm postgres-backup /scripts/pg-restore.sh /backups/authapp-<ts>.dump
set -euo pipefail

: "${PGDATABASE:?PGDATABASE must be set}"

archive="${1:-}"
if [ -z "$archive" ]; then
	echo "usage: pg-restore.sh <archive.dump>" >&2
	exit 2
fi
if [ ! -f "$archive" ]; then
	echo "restore: no such archive: ${archive}" >&2
	exit 1
fi

echo "restore: restoring '${archive}' into database '${PGDATABASE}' (clean replace)"
# --single-transaction so a failed restore rolls back rather than leaving a
# half-restored schema. Ownership is preserved: the app role already exists
# (created by initdb/10-app-role.sh), so its objects come back owned by it.
pg_restore \
	--dbname "$PGDATABASE" \
	--clean --if-exists \
	--single-transaction \
	--exit-on-error \
	"$archive"
echo "restore: done"
