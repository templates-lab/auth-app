#!/usr/bin/env bash
# Hermetic round-trip test for the backup/restore scripts.
#
# The scripts in this directory drive `pg_dump` / `pg_restore`, so a full test
# would normally need a live Postgres. That dependency is exactly why the
# round-trip had only ever been syntax-checked (`bash -n`), never executed. This
# harness removes the dependency: it puts stub `pg_dump` / `pg_restore` binaries
# on PATH and runs the *real* `pg-backup.sh` and `pg-restore.sh` end-to-end, so
# their observable contract — atomic archive rename, retention pruning, and the
# restore flags/exit codes — is actually exercised.
#
#   ./infra/postgres/scripts/test-backup-restore.sh
#
# No Docker or Postgres required. Exits 0 when every assertion passes, non-zero
# on the first failure.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# --- stub the postgres client binaries on PATH ------------------------------
# Both stubs behave just enough for the scripts under test to run: pg_dump
# writes a marker archive at its --file target (so the atomic rename + pruning
# see a real file), pg_restore records the arguments it was handed (so the test
# can assert the restore contract) and succeeds.
bin="$work/bin"
mkdir -p "$bin"

cat >"$bin/pg_dump" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
file=""
for arg in "$@"; do
	case "$arg" in
	--file=*) file="${arg#--file=}" ;;
	esac
done
[ -n "$file" ] || {
	echo "pg_dump stub: missing --file" >&2
	exit 1
}
printf 'PGDMP-stub-archive\n' >"$file"
STUB

cat >"$bin/pg_restore" <<STUB
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\$*" >"$work/restore-args"
STUB

chmod +x "$bin/pg_dump" "$bin/pg_restore"
export PATH="$bin:$PATH"

export PGDATABASE="authapp"

pass=0
fail=0
ok() {
	pass=$((pass + 1))
	printf '  ok   %s\n' "$1"
}
bad() {
	fail=$((fail + 1))
	printf '  FAIL %s\n' "$1" >&2
}

# --- backup: one run writes exactly one archive -----------------------------
echo "test: backup writes a custom-format archive"
b1="$work/backups1"
BACKUP_DIR="$b1" BACKUP_RETENTION=7 "$here/pg-backup.sh" >/dev/null
shopt -s nullglob
archives=("$b1/$PGDATABASE"-*.dump)
shopt -u nullglob
if [ "${#archives[@]}" -eq 1 ]; then
	ok "one archive created"
else
	bad "expected 1 archive, found ${#archives[@]}"
fi
# The atomic-write temp file must not survive a successful run.
if [ -z "$(find "$b1" -name '*.part' -print -quit)" ]; then
	ok "no leftover .part temp file"
else
	bad "atomic-write .part temp file left behind"
fi

# --- backup: retention prunes to the newest N -------------------------------
echo "test: retention keeps only the newest BACKUP_RETENTION archives"
b2="$work/backups2"
mkdir -p "$b2"
# Seed three archives with staggered (older) mtimes; the real run then adds a
# fourth, newest one. With RETENTION=2 the two oldest must be pruned.
for i in 1 2 3; do
	f="$b2/$PGDATABASE-2020010${i}T000000Z.dump"
	printf 'old\n' >"$f"
	touch -d "2020-01-0${i} 00:00:00" "$f"
done
BACKUP_DIR="$b2" BACKUP_RETENTION=2 "$here/pg-backup.sh" >/dev/null
shopt -s nullglob
remaining=("$b2/$PGDATABASE"-*.dump)
shopt -u nullglob
if [ "${#remaining[@]}" -eq 2 ]; then
	ok "pruned down to 2 archives"
else
	bad "expected 2 archives after prune, found ${#remaining[@]}"
fi
# The freshly written archive (newest mtime) must be one of the survivors.
newest="$(ls -1t "$b2/$PGDATABASE"-*.dump | head -1)"
if grep -q 'PGDMP-stub-archive' "$newest"; then
	ok "newest archive is the fresh backup"
else
	bad "fresh backup was pruned instead of an old one"
fi

# --- restore: argument and error handling -----------------------------------
echo "test: restore rejects missing/absent archives"
rc=0
BACKUP_DIR="$b1" "$here/pg-restore.sh" >/dev/null 2>&1 || rc=$?
if [ "$rc" -eq 2 ]; then
	ok "missing argument exits 2"
else
	bad "missing argument: expected exit 2, got $rc"
fi
rc=0
"$here/pg-restore.sh" "$work/does-not-exist.dump" >/dev/null 2>&1 || rc=$?
if [ "$rc" -eq 1 ]; then
	ok "absent archive exits 1"
else
	bad "absent archive: expected exit 1, got $rc"
fi

# --- restore: happy path drives pg_restore with the safe flags --------------
echo "test: restore invokes pg_restore with the clean/transactional contract"
rc=0
"$here/pg-restore.sh" "$newest" >/dev/null 2>&1 || rc=$?
if [ "$rc" -eq 0 ]; then
	ok "restore of a valid archive exits 0"
else
	bad "restore of a valid archive: expected exit 0, got $rc"
fi
args="$(cat "$work/restore-args" 2>/dev/null || true)"
for flag in "--dbname $PGDATABASE" "--clean" "--if-exists" "--single-transaction" "--exit-on-error"; do
	if printf '%s' "$args" | grep -q -- "$flag"; then
		ok "pg_restore received: $flag"
	else
		bad "pg_restore missing: $flag"
	fi
done
if printf '%s' "$args" | grep -qF "$newest"; then
	ok "pg_restore received the archive path"
else
	bad "pg_restore did not receive the archive path"
fi

# --- summary ----------------------------------------------------------------
echo
echo "result: ${pass} passed, ${fail} failed"
[ "$fail" -eq 0 ]
