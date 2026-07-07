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
│   ├── infrastructure/  Adapters implementing domain and payments ports (Postgres, argon2, OIDC)
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

Two `PaymentProvider` adapters ship in `infrastructure`, selected by
`PAYMENT_PROVIDER` — switching between them recompiles no domain or
application logic:

- **`stripe`** (`payments_stripe.rs`) — the reference adapter. Talks to
  Stripe's REST API through the shared `HttpClient` seam (the same one the
  OIDC adapter uses); test mode vs live is only which `STRIPE_SECRET_KEY` you
  set. It maps Stripe's PaymentIntent status strings onto this crate's
  provider-agnostic `PaymentStatus`, and never leaks a Stripe SDK type across
  the trait. Its request-building, response-parsing, status-mapping, and the
  full intent → capture → refund path are unit-tested against a scripted
  transport; only the socket to Stripe itself is unexercised (no live key in
  CI).
- **`fake`** (`payments_fake.rs`) — a deterministic in-memory provider for
  local dev and integration tests, no credentials or network. It is a real,
  env-selectable adapter, not just a test double: outcomes are driven by the
  amount's cents (like a gateway's test cards) — cents `01` force a decline,
  cents `02` force a timeout, anything else succeeds — and created intents are
  tracked so capture/refund/status stay coherent.

| Variable            | Meaning                                                     |
| ------------------- | ---------------------------------------------------------- |
| `PAYMENT_PROVIDER`  | `stripe` \| `fake` \| unset/`none` (disabled)              |
| `STRIPE_SECRET_KEY` | Required for `stripe` (`sk_test_...` or `sk_live_...`)     |
| `STRIPE_API_BASE`   | Optional; defaults to `https://api.stripe.com`             |

Still to come as their own beads: signature-verified idempotent webhooks and
a transactions view in the admin panel. The payment HTTP surface that wires
the selected provider into routes lands with those.

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

## Authentication audit trail

Every login attempt (success, failure, or lockout) and every logout is
recorded to `admin_audit_events` (Postgres): event type, the resolved admin
id when there is one, the submitted email (kept even when it matched no
account — that is exactly the case an admin id can't identify), client IP,
`User-Agent`, and timestamp. No password, session token, or CSRF token is
ever recorded — `NewAuditEvent` simply has no field to put one in, so that is
a property of the type rather than a rule callers have to remember. Recording
is best-effort: an outage in the audit store logs a warning but never blocks
a real login or logout.

```sh
curl http://localhost:8080/audit/events?limit=20 \
  --cookie "session=$SESSION"
# 200 [{"event_type":"login_succeeded","admin_id":"...","email_attempted":"...", ...}, ...]
# 401 {"error":"unauthorized"} without a valid session
# 403 {"error":"forbidden"} with a valid session that isn't `admin` (see Roles below)
```

Refresh-token events, OAuth account linking, and password-change events join
this trail once those features exist (`AuditEventType` is a closed set that
gains a variant per new feature, not a free-form string); the admin panel's
own audit *view* is a separate frontend task — this backend query endpoint is
the surface it will call.

## Roles (RBAC)

Every account has a `role` (`admin_users.role`, default and — for now, since
there is no "create user" endpoint yet — only ever `admin`, assigned by
bootstrap). The role rides along on the session as a snapshot, the same
trade-off already made for the CSRF token: it is set once at login and does
not change mid-session, so changing an account's role takes effect on that
account's *next* login, not immediately. `Role` is a validated string, not a
closed Rust enum — adding a role (`"editor"`, say) is a data change, not a
recompile.

- **`GET /auth/me`** — any authenticated session — reports `{admin_id, role}`,
  the surface frontend guards call to decide which routes/actions to show.
- **`crates/api/src/rbac.rs`**'s `require_role` middleware gates a specific
  endpoint to a specific role, `403` otherwise. `GET /audit/events` is the
  first example (`Role::admin()`); gating a new endpoint to a new role is one
  `.layer(...)` line, not a structural change.

```sh
curl http://localhost:8080/auth/me --cookie "session=$SESSION"
# 200 {"admin_id":"...","role":"admin"}
```

## OAuth2 / OIDC sign-in

Admins can also sign in through an external identity provider using the
OAuth2 authorization-code flow with **PKCE**. The flow is behind the
`OAuthProvider` trait (`crates/domain/src/oauth.rs`); a generic, config-driven
OIDC adapter (`crates/infrastructure/src/oauth_provider.rs`) serves any
standard OIDC provider, so **adding a provider is configuration, not code**. A
non-OIDC provider (GitHub, say) is a new `impl OAuthProvider` next to it — the
trait, not the struct, is the extension point.

Two routes (both public — no session exists yet):

```sh
# 1. Start: 303 redirect to the provider's authorize URL (with state, nonce,
#    and the PKCE S256 challenge). Unknown provider → 404.
GET /auth/oauth/{provider}/start

# 2. Callback: the provider sends the browser back here. The server validates
#    state (one-shot — a replay finds nothing), exchanges the code with the
#    PKCE verifier, validates the id_token's nonce/iss/aud/exp, resolves the
#    identity, issues a session, and 303-redirects to OAUTH_SUCCESS_REDIRECT
#    (or OAUTH_FAILURE_REDIRECT?error=oauth on any failure).
GET /auth/oauth/{provider}/callback?state=...&code=...
```

Security properties, each enforced structurally:

- **state** is a one-shot server-side value (`oauth_pending_authorizations`,
  consumed by a `DELETE ... RETURNING`), so a callback cannot be replayed.
- **PKCE** — the verifier never leaves the server; only its `S256` challenge
  is sent in the authorize URL.
- **nonce** — the id_token must echo the nonce the flow generated.
- **Tokens never reach the frontend** — the provider adapter hands the
  application only an `OAuthIdentity { provider, subject, email }`; there is no
  access token or id_token in any type the delivery layer can see, let alone
  leak.
- **No silent admin provisioning** — an external identity signs in only if it
  is already linked, or an admin account with its (provider-verified) email
  exists (then it is linked). An unknown identity is refused.

External identities are stored in their own table (`admin_oauth_identities`,
`(provider, subject) -> admin_id`).

Configuration (all optional; unset `OAUTH_PROVIDERS` disables OAuth entirely):

| Variable                        | Meaning                                             |
| ------------------------------- | --------------------------------------------------- |
| `OAUTH_PROVIDERS`               | Comma-separated provider ids to enable (e.g. `google`) |
| `OAUTH_REDIRECT_BASE`           | External origin for the callback URL (default `http://localhost:8080`) |
| `OAUTH_SUCCESS_REDIRECT`        | Path after a successful sign-in (default `/`)       |
| `OAUTH_FAILURE_REDIRECT`        | Path after a failed sign-in (default `/login`)      |
| `OAUTH_<ID>_CLIENT_ID`          | The provider's OAuth client id                      |
| `OAUTH_<ID>_CLIENT_SECRET`      | The provider's OAuth client secret                  |
| `OAUTH_<ID>_AUTH_ENDPOINT`      | Authorization endpoint                              |
| `OAUTH_<ID>_TOKEN_ENDPOINT`     | Token endpoint                                      |
| `OAUTH_<ID>_USERINFO_ENDPOINT`  | Userinfo endpoint                                   |
| `OAUTH_<ID>_ISSUER`             | Expected `iss` in the id_token                      |
| `OAUTH_<ID>_SCOPES`             | Comma-separated scopes (default `openid,email`)     |

> **Remaining hardening:** the adapter validates the id_token's claims
> (nonce/iss/aud/exp) but does not yet verify its RS256 signature against the
> provider's JWKS. This is safe as shipped because the id_token arrives
> directly from the token endpoint over TLS (not via the browser) and the
> identity of record comes from the bearer-authenticated userinfo call; adding
> JWKS signature verification is the documented next step.

[`PaymentProvider`]: crates/payments/src/provider.rs
