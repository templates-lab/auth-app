# Deployment

Zero to a running stack (bead authapp-242382). The stack is composed of a
Traefik edge, the Rust API, the SolidJS web app, and Postgres — wired by the
root `compose.yml` with `dev` and `prod` profiles. See the per-component notes
in [`infra/traefik`](../infra/traefik/README.md) and
[`infra/postgres`](../infra/postgres/README.md).

## Prerequisites

- Docker with the Compose plugin (`docker compose version`).
- For local frontend/backend work outside containers: Node ≥ 20 + pnpm, and a
  Rust toolchain (see the root [README](../README.md)). Not required just to run
  the stack.

## Configuration

Every variable the stack reads is documented in
[`.env.example`](../.env.example). Copy it and edit:

```bash
cp .env.example .env
```

`docker compose` auto-loads `./.env`. `.env` is git-ignored — never commit real
secrets. The example values are dev placeholders; change every password before
any real deployment.

## Local development

Bring up the whole stack, building the api/web images locally:

```bash
docker compose --profile dev up --build
```

This starts Postgres (with a healthcheck), the API (which waits for a healthy
database, runs its migrations, and creates the least-privilege app role), the
web app, and Traefik. A one-shot `bootstrap` service seeds the first admin from
`ADMIN_BOOTSTRAP_EMAIL` / `ADMIN_BOOTSTRAP_PASSWORD`.

The dev override (`compose.override.yml`, auto-loaded) also publishes Postgres
on `127.0.0.1:5432` and the API on `127.0.0.1:8080` for direct access:

```bash
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/health   # 200 (liveness)
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8080/ready    # 200 when the DB is reachable
```

Sign in at the web app (through Traefik on `https://$APP_DOMAIN`, or hit the API
directly) with the bootstrapped admin. Tear down with `docker compose --profile
dev down` (add `-v` to also drop the database volume).

## Production

Production uses versioned, registry-hosted images (never a local build) and
Docker secrets. It is the base `compose.yml` plus the `compose.prod.yml` overlay
— note the explicit `-f` list, which excludes the dev override:

```bash
docker compose -f compose.yml -f compose.prod.yml --profile prod up -d
```

### 1. Build and push versioned images

```bash
docker build -f infra/docker/api.Dockerfile -t ghcr.io/your-org/auth-app-api:v1.0.0 .
docker build -f infra/docker/web.Dockerfile -t ghcr.io/your-org/auth-app-web:v1.0.0 .
docker push ghcr.io/your-org/auth-app-api:v1.0.0
docker push ghcr.io/your-org/auth-app-web:v1.0.0
```

Set `API_IMAGE` / `WEB_IMAGE` in `.env` (or the deploy environment) to those tags.

### 2. Provision secrets

`compose.prod.yml` reads secrets from `./secrets/*` (git-ignored). Create them
out of band on the host — never in the repo:

```bash
mkdir -p secrets
printf '%s' "$(openssl rand -base64 24)" > secrets/postgres_superuser_password
printf '%s' "postgres://authapp_app:$(openssl rand -base64 24 | tr -d /=+)@postgres:5432/authapp?sslmode=disable" \
  > secrets/database_url
```

Postgres reads its superuser password via `POSTGRES_PASSWORD_FILE`; the API
reads its connection string via `DATABASE_URL_FILE` (see
`crates/infrastructure/src/db.rs`). No credential is ever a plain environment
variable in production.

### 3. Domain and TLS

Point `APP_DOMAIN`'s DNS at the host, set a real `ACME_EMAIL`, and open ports
`80`/`443`. Traefik obtains and renews a Let's Encrypt certificate for
`APP_DOMAIN` automatically; only Traefik publishes ports — the API and database
stay on internal networks (the database is unreachable from the edge network).

### 4. Bring it up and verify

```bash
docker compose -f compose.yml -f compose.prod.yml --profile prod up -d
docker compose -f compose.yml -f compose.prod.yml --profile prod ps
```

Create the first admin once (idempotent — it refuses to run once any admin
exists):

```bash
docker compose -f compose.yml -f compose.prod.yml --profile prod \
  run --rm --entrypoint /usr/local/bin/server api bootstrap-admin
```

Health checks for an orchestrator or uptime monitor: `GET /api/health`
(liveness) and `GET /api/ready` (readiness — reflects database connectivity).

## Backups

Enable the scheduled-backup sidecar with the `backup` profile, and see
[`infra/postgres/README.md`](../infra/postgres/README.md) for the manual
backup/restore commands and a verified round-trip procedure:

```bash
docker compose -f compose.yml -f compose.prod.yml --profile prod --profile backup up -d
```

## Observability

Logs are structured JSON (one object per line) with a `request_id` per request,
which is also returned in the `x-request-id` response header — set `RUST_LOG` to
adjust verbosity (default `info`). Ship container stdout to your log collector.

## Security

Before going live, work through [`SECURITY.md`](SECURITY.md) — the security
baseline checklist (mapped to OWASP ASVS). It marks what the template already
covers and the per-instance items you must complete: unique secrets per
environment, real provider keys, resource-level authorization for any new
domains, and your data-protection decisions.
