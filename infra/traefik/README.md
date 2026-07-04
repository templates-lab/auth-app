# Traefik edge

Traefik is the single entry point for the stack. It terminates TLS, routes
traffic to the web SPA and the API by Docker labels, and applies the edge
middlewares (security headers, compression, rate limiting).

## Files

| File | Role |
| --- | --- |
| `traefik.yml` | Static config: entry points, ACME resolver, providers. Read at startup. |
| `dynamic/middlewares.yml` | Reusable middlewares referenced from labels as `<name>@file`. Hot-reloaded. |
| `dynamic/tls.yml` | TLS options: minimum version, cipher suites, strict SNI. Hot-reloaded. |
| `docker-compose.yml` | The edge stack — `traefik`, `api`, `web` — with the routing labels. |

## Routing

Two routers are declared on the service labels, both on the `websecure` (`:443`)
entry point with automatic Let's Encrypt certificates:

- **web** — `Host(<APP_DOMAIN>)`, priority `1` (catch-all). Serves the SPA on
  `/`. Middlewares: `web-chain` (security headers + compression).
- **api** — `Host(<APP_DOMAIN>) && PathPrefix(/api)`, priority `100` so it wins
  over the web catch-all. Middlewares: `api-chain` (strip `/api` → rate limit →
  security headers → compression).

The `api-strip` middleware removes the `/api` prefix, so the backend stays
prefix-agnostic: a request to `https://<domain>/api/health` reaches the Rust
service as `/health`.

The `web` (`:80`) entry point does nothing but permanently redirect to HTTPS and
answer the ACME HTTP-01 challenge.

## TLS

Certificates are issued and renewed automatically by the `letsencrypt` resolver
and persisted in the `letsencrypt` volume (`/letsencrypt/acme.json`).

- **HTTP-01** (default) — needs only ports 80/443 reachable.
- **DNS-01** (wildcards, or when port 80 is unreachable) — swap the
  `httpChallenge` block in `traefik.yml` for the commented `dnsChallenge` block
  and supply the DNS provider credentials via the environment.

## Environment

| Variable | Required | Default | Purpose |
| --- | --- | --- | --- |
| `ACME_EMAIL` | yes | — | Registration/contact address on issued certificates. Startup fails if unset. |
| `APP_DOMAIN` | no | `localhost` | Host the routers match. Use a real DNS name in production. |

## Run

Standalone (Traefik config + routing only; the `api`/`web` images are built by
`infra/docker/*.Dockerfile`):

```sh
ACME_EMAIL=you@example.com APP_DOMAIN=auth.example.com \
  docker compose -f infra/traefik/docker-compose.yml up
```

Validate the routing/label wiring without starting anything:

```sh
docker compose -f infra/traefik/docker-compose.yml config
```

> Local note: browsers reject Let's Encrypt certificates for `localhost`. For
> local runs, point `APP_DOMAIN` at a resolvable name or expect the staging/
> self-signed fallback; production uses a real domain with public DNS.
