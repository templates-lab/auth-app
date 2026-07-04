#!/usr/bin/env bash
# One-shot logical backup: a compressed, custom-format pg_dump archive written
# atomically to $BACKUP_DIR, then the directory is pruned to the most recent
# $BACKUP_RETENTION archives. Connection settings come from the standard libpq
# environment variables (PGHOST, PGUSER, PGPASSWORD, PGDATABASE).
#
# Run it on a schedule via the backup sidecar (backup-scheduler.sh), or manually:
#   docker compose -f infra/postgres/docker-compose.yml --profile backup \
#     run --rm postgres-backup /scripts/pg-backup.sh
set -euo pipefail

BACKUP_DIR="${BACKUP_DIR:-/backups}"
RETENTION="${BACKUP_RETENTION:-7}"
: "${PGDATABASE:?PGDATABASE must be set}"

mkdir -p "$BACKUP_DIR"

timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
target="${BACKUP_DIR}/${PGDATABASE}-${timestamp}.dump"
tmp="${target}.part"

echo "backup: dumping database '${PGDATABASE}' -> ${target}"
# --format=custom yields a compressed archive restorable with pg_restore.
# Write to a temp name and rename so a crash never leaves a half-written archive
# that looks complete to the retention/restore logic.
pg_dump --format=custom --file="$tmp" "$PGDATABASE"
mv "$tmp" "$target"
echo "backup: wrote $(du -h "$target" | cut -f1) ${target}"

# Prune: keep the newest $RETENTION archives, delete older ones.
count=0
while IFS= read -r archive; do
	count=$((count + 1))
	if [ "$count" -gt "$RETENTION" ]; then
		echo "backup: pruning old archive ${archive}"
		rm -f "$archive"
	fi
done <<EOF
$(ls -1t "${BACKUP_DIR}/${PGDATABASE}"-*.dump 2>/dev/null || true)
EOF

echo "backup: done (retention ${RETENTION})"
