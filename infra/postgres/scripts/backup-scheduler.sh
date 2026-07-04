#!/usr/bin/env bash
# Entrypoint for the postgres-backup sidecar: take a backup now, then every
# $BACKUP_INTERVAL_SECONDS. A failed run is logged and retried on the next tick
# rather than crashing the sidecar, so one transient database blip never stops
# the schedule.
set -euo pipefail

INTERVAL="${BACKUP_INTERVAL_SECONDS:-86400}"

echo "backup-scheduler: starting (interval=${INTERVAL}s, retention=${BACKUP_RETENTION:-7}, dir=${BACKUP_DIR:-/backups})"
while true; do
	if ! /scripts/pg-backup.sh; then
		echo "backup-scheduler: backup failed; will retry in ${INTERVAL}s" >&2
	fi
	sleep "$INTERVAL"
done
