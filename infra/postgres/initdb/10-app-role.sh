#!/usr/bin/env bash
# Create the least-privilege application role and hand it ownership of the
# working schema.
#
# This runs once, during first cluster initialisation (docker-entrypoint-initdb.d
# only executes when the data directory is empty), as the bootstrap superuser
# ($POSTGRES_USER) connected to the application database ($POSTGRES_DB).
#
# The application connects as $APP_DB_USER, which is deliberately NOT a
# superuser and cannot create roles or databases. It owns the `public` schema of
# its own database so the app's migrations (which CREATE tables) succeed, but
# that authority is confined to this one database — it holds no cluster-wide
# privileges. The trusted `pgcrypto` extension is pre-installed here by the
# superuser so the app's baseline migration (`CREATE EXTENSION IF NOT EXISTS
# pgcrypto`) is a privilege-free no-op.
set -euo pipefail

: "${APP_DB_USER:?APP_DB_USER must be set}"
: "${APP_DB_PASSWORD:?APP_DB_PASSWORD must be set}"

psql -v ON_ERROR_STOP=1 \
	--username "$POSTGRES_USER" \
	--dbname "$POSTGRES_DB" \
	--set app_user="$APP_DB_USER" \
	--set app_password="$APP_DB_PASSWORD" \
	--set app_db="$POSTGRES_DB" <<-'SQL'
	-- Application login role: idempotent create (CREATE ROLE has no IF NOT
	-- EXISTS, so guard it and \gexec the generated statement). NOSUPERUSER /
	-- NOCREATEDB / NOCREATEROLE keep it powerless at the cluster level; the
	-- password is passed as a quoted literal (%L) to survive special characters.
	SELECT format(
	  'CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS PASSWORD %L',
	  :'app_user', :'app_password'
	)
	WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'app_user')
	\gexec

	-- Let the app connect to its database.
	GRANT CONNECT ON DATABASE :"app_db" TO :"app_user";

	-- Pre-install the trusted extension as the superuser so the app never needs
	-- extension-creation privileges.
	CREATE EXTENSION IF NOT EXISTS pgcrypto;

	-- Give the app role ownership of the working schema so its migrations can
	-- create objects, and revoke the implicit CREATE that PUBLIC would otherwise
	-- have on `public` (defence in depth).
	ALTER SCHEMA public OWNER TO :"app_user";
	REVOKE CREATE ON SCHEMA public FROM PUBLIC;
	GRANT USAGE, CREATE ON SCHEMA public TO :"app_user";

	-- Let the app create new schemas in its own database: a migration adds the
	-- `payments` bounded-context schema (0004_payments_schema), so the role that
	-- runs the migrations at startup needs database-level CREATE. This stays
	-- confined to this one database — the role remains NOSUPERUSER/NOCREATEDB/
	-- NOCREATEROLE and holds no cluster-wide authority.
	GRANT CREATE ON DATABASE :"app_db" TO :"app_user";
	SQL

echo "init: created least-privilege role '${APP_DB_USER}' and granted it ownership of schema public in '${POSTGRES_DB}'"
