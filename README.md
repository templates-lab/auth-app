# auth-app

Authentication app template — a modular monorepo with a Rust backend and a
web frontend, deployable behind Traefik with Postgres.

## Layout

```
.
├── apps/          Frontend applications (pnpm workspace)
│   └── web/       Vite + SolidJS admin shell (@auth-app/web)
├── packages/      Shared + feature packages (pnpm workspace)
│   ├── shared/            Framework-agnostic shared utilities (@auth-app/shared)
│   ├── feature-kit/       Feature contract for the shell (@auth-app/feature-kit)
│   ├── feature-dashboard/ Dashboard feature (@auth-app/feature-dashboard)
│   └── feature-users/     Users feature (@auth-app/feature-users)
├── crates/        Rust backend (Cargo workspace, hexagonal architecture)
│   ├── domain/          Auth/session business model and ports — no framework deps
│   ├── payments/        Payments business model and ports — a second, independent domain
│   ├── application/     Use cases orchestrating the domain
│   ├── infrastructure/  Adapters implementing domain and payments ports
│   ├── api/             HTTP boundary (axum router)
│   ├── server/          Composition root — the `server` binary
│   ├── testkit/         Ephemeral-Postgres integration test harness (dev-only)
│   └── xtask/           Workspace automation (`cargo xtask`)
├── infra/         Deployment
│   ├── docker/    Dockerfiles
│   └── traefik/   Traefik routing and TLS
└── docs/          Architecture and operations docs
```

The Rust crates form a Cargo workspace rooted at `Cargo.toml`; the frontend
apps and packages form a separate pnpm workspace. The two toolchains stay
decoupled on purpose so a change on one side never forces a rebuild of the
other.

Shared frontend tooling lives at the repo root and is inherited by every
package:

- `tsconfig.base.json` — base TypeScript config each package extends
- `eslint.config.js` — flat ESLint config (ESLint 9), auto-discovered by every package
- `.prettierrc.json` — Prettier formatting rules
- `pnpm-workspace.yaml` — workspace package globs

Shared Rust conventions live at the root: compilation profiles and lints in
`Cargo.toml`, formatting in `rustfmt.toml`, and Clippy config in `clippy.toml`.

## Frontend architecture: an app shell composed of feature packages

The web app (`@auth-app/web`) is a SolidJS **shell** — it owns the admin chrome
(a responsive sidebar, header, and content area, in `apps/web/src/shell/`) and
the router, but no product screens. Each product area is a self-contained
**feature package** under `packages/` that exposes a `FeatureModule`:

- `@auth-app/feature-kit` defines the contract: `FeatureModule` (the routes and
  sidebar entries a feature contributes) plus the `defineFeature` helper.
- `@auth-app/feature-dashboard` and `@auth-app/feature-users` are example
  features. Each declares its own routes and navigation and depends only on
  `feature-kit` and `shared` — never on another feature.

The shell discovers features through a single registry
(`apps/web/src/shell/registry.ts`): it mounts every feature's routes with
`@solidjs/router` and derives the sidebar from their nav entries. **Adding a
feature is additive** — create the package, add it as a dependency of
`@auth-app/web`, and list it in the registry. No existing feature or layout
code changes, satisfying the open/closed boundary between features.

Feature packages ship Solid source (`.tsx`) consumed directly by the app's Vite
build (`vite-plugin-solid`); only `shared` and `feature-kit` compile to `dist`.

## Requirements

- Node.js >= 20
- pnpm (pinned via `packageManager`; use `corepack enable` to activate it)
- Rust (stable toolchain)
- Docker — only needed to run `cargo test`; the backend's integration tests
  spin up their own ephemeral Postgres containers (see below)

## Getting started

Frontend (pnpm workspace):

```bash
corepack enable            # activates the pinned pnpm version
pnpm install               # resolve the whole workspace
pnpm -r run build          # build every package (topological order)
pnpm -r run lint           # lint every package
pnpm -r run test           # test every package
pnpm --filter @auth-app/web dev   # run the web app
```

Root convenience scripts (`pnpm dev`, `pnpm build`, `pnpm test`, `pnpm lint`,
`pnpm format`) fan out across the workspace.

Backend (Cargo workspace):

```sh
cargo build          # build the Rust workspace
cargo clippy         # lint
cargo fmt            # format (rules in rustfmt.toml)
cargo xtask help     # list workspace tasks
cargo run -p server  # start the HTTP server
```

The server binary is the composition root: it reads its configuration from the
environment (`APP_HOST`, default `0.0.0.0`; `APP_PORT`, default `8080`), builds
the infrastructure adapters, injects them into the application services, and
serves the API router.

### Integration tests

`cargo test` runs two kinds of tests: pure unit tests (no I/O) alongside each
module's source, and Postgres-backed integration tests under
`crates/infrastructure/tests/*_it.rs`. The latter need no manual database
setup — the `testkit` crate (`crates/testkit`) starts a fresh, disposable
Postgres container per test via [testcontainers](https://testcontainers.com/),
applies every embedded migration against it, and hands the test a connected
pool. Each test gets its own container, so tests never share schema state and
run safely in parallel. The only requirement is a running Docker daemon —
nothing to start or tear down by hand, and nothing left behind afterward.

Adding an integration-test suite for a new module follows the same shape:

```rust
#[tokio::test]
async fn my_repo_round_trips() {
    let db = testkit::spawn_test_db().await;
    let repo = PgMyRepository::new(db.pool.clone());
    // ... exercise `repo` against the real, migrated database.
}
```

## Admin authentication

The backend ships an administrator login built through the hexagonal layers:
`POST /auth/login` verifies an email/password pair with **argon2id** (OWASP cost
parameters by default), a **constant-work** path so a nonexistent account and a
wrong password are indistinguishable by response or timing, and **progressive
lockout** applied independently per account and per client IP.

```sh
curl -X POST http://localhost:8080/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"admin@example.com","password":"..."}'
# 200 {"admin_id":"..."} on success, plus Set-Cookie: session=... and csrf=...
# 401 {"error":"invalid_credentials"} for a wrong password OR unknown account
# 429 {"error":"too_many_attempts"} (+ Retry-After) when locked out
```

### Sessions and CSRF

A successful login issues a brand-new, server-side session persisted in
Postgres — never a reused or client-signed token — which is what makes login
itself a session rotation. Two cookies are set, both `Secure` and
`SameSite=Strict`:

- `session` — `HttpOnly`, the bearer token every subsequent request
  authenticates with. Never readable by client-side script.
- `csrf` — readable by client-side script on purpose: mirror its value into an
  `X-CSRF-Token` header on every mutating request (`POST`/`PUT`/`PATCH`/`DELETE`).
  A mismatched or missing header is rejected with `403` before the handler
  runs; `GET`/`HEAD`/`OPTIONS` are exempt.

A session dies at whichever comes first: `SESSION_IDLE_TIMEOUT_SECS` of
inactivity, or `SESSION_ABSOLUTE_TIMEOUT_SECS` since it was issued.
`POST /auth/logout` (itself CSRF-protected) revokes the session server-side and
clears both cookies:

```sh
curl -X POST http://localhost:8080/auth/logout \
  --cookie "session=$SESSION; csrf=$CSRF" \
  -H "x-csrf-token: $CSRF"
# 204 on success (idempotent — an already-expired session still clears cookies)
# 401 {"error":"unauthorized"} for a missing/invalid/expired session
# 403 {"error":"csrf_mismatch"} for a missing/mismatched CSRF header
```

### Bootstrapping the first admin

No password is ever committed to the repository. The first administrator is
seeded from run-time secrets by a one-shot subcommand, which no-ops once any
admin exists:

```sh
ADMIN_BOOTSTRAP_EMAIL=admin@example.com \
ADMIN_BOOTSTRAP_PASSWORD='a-strong-secret-passphrase' \
  cargo run -p server -- bootstrap-admin
```

### Configuration

All of the following are optional and default to secure values; a
present-but-unparseable value fails fast at startup.

| Variable                           | Default | Meaning                            |
| ---------------------------------- | ------- | ---------------------------------- |
| `ADMIN_PASSWORD_MIN_LENGTH`        | `12`    | Minimum password length            |
| `ADMIN_PASSWORD_REQUIRE_UPPERCASE` | `true`  | Require an uppercase letter        |
| `ADMIN_PASSWORD_REQUIRE_LOWERCASE` | `true`  | Require a lowercase letter         |
| `ADMIN_PASSWORD_REQUIRE_DIGIT`     | `true`  | Require a digit                    |
| `ADMIN_PASSWORD_REQUIRE_SYMBOL`    | `false` | Require a symbol                   |
| `ADMIN_LOCKOUT_MAX_ATTEMPTS`       | `5`     | Failures before lockout engages    |
| `ADMIN_LOCKOUT_BASE_SECONDS`       | `60`    | First lock duration (then doubles) |
| `ADMIN_LOCKOUT_MAX_SECONDS`        | `3600`  | Ceiling for the lock duration      |
| `ARGON2_MEMORY_KIB`                | `19456` | argon2id memory cost (KiB)         |
| `ARGON2_ITERATIONS`                | `2`     | argon2id iterations                |
| `ARGON2_PARALLELISM`               | `1`     | argon2id parallelism               |
| `SESSION_IDLE_TIMEOUT_SECS`        | `1800`  | Session dies after this much inactivity |
| `SESSION_ABSOLUTE_TIMEOUT_SECS`    | `43200` | Session dies this long after login, regardless of activity |
| `LOGIN_RATE_LIMIT_MAX_REQUESTS`    | `10`    | Login attempts allowed per window, per IP and per account |
| `LOGIN_RATE_LIMIT_WINDOW_SECS`     | `60`    | The login rate limit's window duration |

## Payments

The `payments` crate is a second, independent domain (its own bounded
context — it never depends on, or is depended on by, the auth `domain`): the
[`PaymentProvider`] trait (`create_intent`, `capture`, `refund`,
`get_status`) and the payment state machine
(`Created → RequiresAction/Authorized → Captured → PartiallyRefunded/Refunded`,
with `Failed`/`Canceled` reachable early and terminal). No payment-provider
SDK type ever crosses into this crate's public API — swapping providers, or
adding a second one, touches only a new adapter behind [`PaymentProvider`].

State only ever changes through `PaymentRepository::transition`, which is
atomic and optimistic-concurrency-guarded: it moves a payment from an
`expected_current` status to the next one and appends a row to its history in
one transaction, in Postgres's own `payments` schema
(`payments.payments` / `payments.payment_status_history`). A caller whose
`expected_current` no longer matches (another transition already won) gets
`PaymentRepositoryError::Conflict` back rather than a corrupted state machine.

This bead intentionally stops at the trait, the model, and the schema — no
concrete provider, no webhook handling, and no admin UI yet. Those land as
their own beads: a Stripe (and fake, for tests) provider, signature-verified
idempotent webhooks, and a transactions view in the admin panel.

## Security headers and CORS

Two layers, split by what each is good at:

- **Traefik** (`infra/traefik/dynamic/middlewares.yml`, `security-headers`)
  sets the static, per-route response headers: HSTS (1 year,
  subdomains, preload), `X-Content-Type-Options: nosniff`, a strict
  `Content-Security-Policy` (no `unsafe-inline`/`unsafe-eval` anywhere —
  `default-src 'self'` plus per-directive `'self'`), and `frame-ancestors
  'none'` (paired with `X-Frame-Options: DENY` for older browsers).
- **The API** (`crates/api/src/cors.rs`) owns CORS, because it needs
  per-request `Origin` matching and preflight (`OPTIONS`) handling that a
  static header cannot express. `CORS_ALLOWED_ORIGINS` (comma-separated exact
  origins, e.g. `https://admin.example.com,http://localhost:5173`) is the
  allowlist; unset or empty allows no cross-origin request at all — there is
  no wildcard fallback at any point. Credentialed requests (the session/CSRF
  cookies) are allowed only for origins on that list.

A single-origin deployment (the default Traefik routing — web on `/`, API on
`/api`, same host) needs `CORS_ALLOWED_ORIGINS` unset: same-origin requests
are never subject to CORS in the first place.

## Rate limiting

Two layers again:

- **Traefik** (`infra/traefik/dynamic/middlewares.yml`, `api-ratelimit`)
  applies a blunt, global limit across the whole API (~50 req/s per client
  IP, bursting to 100) — it protects the service regardless of route, but it
  cannot see the request body.
- **The app** (`crates/api/src/rate_limit.rs`) adds a finer-grained,
  in-memory, fixed-window limiter for `/auth/login`, applied *independently*
  per client IP and per submitted account email (`LOGIN_RATE_LIMIT_MAX_REQUESTS`
  per `LOGIN_RATE_LIMIT_WINDOW_SECS`, defaults `10`/`60`). This is a distinct
  mechanism from the account/IP *lockout* in [Admin authentication](#admin-authentication):
  lockout only counts failed attempts and persists in Postgres; the rate
  limiter counts every attempt (successful or not) and lives only in that
  process's memory — a defense-in-depth layer behind Traefik's shared,
  cross-replica limit, not a replacement for it. A rejected request is logged
  (`login: rate limit exceeded for ...`) and answered `429` with
  `Retry-After`, before any credential work happens. The same `RateLimiter`
  type is meant to be reused for payment webhooks once that endpoint exists.

[`PaymentProvider`]: crates/payments/src/provider.rs
