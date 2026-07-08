# Security baseline

The security posture this template ships with, as an actionable checklist mapped
to [OWASP ASVS](https://owasp.org/www-project-application-security-verification-standard/)
Level 1–2. Read it when instantiating the template: some items are **covered**
by the template, others are **review-per-instance** — they depend on your
domains, providers, and secrets and only you can complete them.

Legend: ✅ covered by the template · 🔶 review per instance · ⬜ not in scope.

## Authentication & credentials

| Item                                                                                       | Status | ASVS        |
| ------------------------------------------------------------------------------------------ | ------ | ----------- |
| Passwords hashed with Argon2id (OWASP params, salt in the PHC string)                      | ✅     | 2.4.1       |
| Constant-time credential verification; identical error for bad password vs unknown account | ✅     | 2.2.1 / 3.2 |
| Progressive per-account and per-IP lockout on failed logins                                | ✅     | 2.2.1       |
| App-level rate limiting on login (per IP and per account)                                  | ✅     | 2.2.1       |
| Password policy (length + character classes), configurable                                 | ✅     | 2.1.x       |
| First admin created out-of-band (`bootstrap-admin`), never a default password              | ✅     | 2.1.x       |
| OAuth2/OIDC with PKCE (S256), state + nonce validation                                     | ✅     | 2.x / 3.5   |
| Breached-password / MFA                                                                    | 🔶     | 2.1.7 / 2.8 |

## Sessions & CSRF

| Item                                                                                                         | Status | ASVS        |
| ------------------------------------------------------------------------------------------------------------ | ------ | ----------- |
| Server-side sessions; opaque token, revocable server-side ([ADR 0003](adr/0003-server-sessions-over-jwt.md)) | ✅     | 3.2         |
| `session` cookie is `HttpOnly` + `Secure` + `SameSite=Strict`                                                | ✅     | 3.4.1–3.4.3 |
| Idle and absolute session timeouts                                                                           | ✅     | 3.3.1–3.3.2 |
| New session issued on each login (no fixation)                                                               | ✅     | 3.2.1       |
| CSRF: double-submit `csrf` cookie + `X-CSRF-Token` header on mutations                                       | ✅     | 4.2.2       |

## Access control

| Item                                                                    | Status | ASVS  |
| ----------------------------------------------------------------------- | ------ | ----- |
| Every protected route requires a valid session (deny by default)        | ✅     | 4.1.1 |
| Role-based checks (`require_role`) on privileged actions (e.g. refunds) | ✅     | 4.1.3 |
| Your own resource-level authorization for new domains                   | 🔶     | 4.2.1 |

## Input handling & errors

| Item                                                                          | Status | ASVS  |
| ----------------------------------------------------------------------------- | ------ | ----- |
| One typed error shape; validation is `422` with per-field detail              | ✅     | 5.1.x |
| Internal errors never leak their cause to the client (logged with a trace id) | ✅     | 7.4.1 |
| Parameterized SQL only (sqlx); no string-built queries                        | ✅     | 5.3.4 |
| Validation/encoding for any new user-supplied input you add                   | 🔶     | 5.1.x |

## Transport & headers

| Item                                                                           | Status | ASVS   |
| ------------------------------------------------------------------------------ | ------ | ------ |
| TLS terminated at Traefik (Let's Encrypt, auto-renew)                          | ✅     | 9.1.1  |
| HSTS and a strict Content-Security-Policy at the edge                          | ✅     | 14.4.x |
| CORS is an explicit allowlist, never a wildcard                                | ✅     | 14.5.3 |
| Set `APP_DOMAIN`, `ACME_EMAIL`, and `CORS_ALLOWED_ORIGINS` for your deployment | 🔶     | 9.1.1  |

## Secrets & configuration

| Item                                                                                        | Status | ASVS   |
| ------------------------------------------------------------------------------------------- | ------ | ------ |
| No secret committed; `.env` and `/secrets/` git-ignored; gitleaks in CI                     | ✅     | 14.3.2 |
| Fail-fast at startup with a clear message when a required secret is missing                 | ✅     | 14.1.x |
| Production reads secrets via Docker secrets (`*_FILE`), not env vars                        | ✅     | 6.4.1  |
| Provision real, unique secrets for every environment (see `.env.example` / `DEPLOYMENT.md`) | 🔶     | 6.4.x  |

## Data & database

| Item                                                               | Status | ASVS  |
| ------------------------------------------------------------------ | ------ | ----- |
| App connects as a least-privilege role (no superuser)              | ✅     | 1.2.1 |
| Database on a private network, never published to the edge or host | ✅     | 1.2.x |
| Persistent volume + scheduled, tested backup/restore               | ✅     | —     |
| Encryption at rest / PII handling for your data                    | 🔶     | 6.1.x |

## Payments

| Item                                                                                                    | Status | ASVS  |
| ------------------------------------------------------------------------------------------------------- | ------ | ----- |
| Provider behind a port; no SDK types in the core ([ADR 0004](adr/0004-provider-agnostic-boundaries.md)) | ✅     | —     |
| Webhooks signature-verified, idempotent, and audited                                                    | ✅     | —     |
| Amounts are integer minor units (no float)                                                              | ✅     | —     |
| Real provider keys + a real webhook secret for your account                                             | 🔶     | 6.4.x |

## Supply chain & CI

| Item                                                                                    | Status | ASVS   |
| --------------------------------------------------------------------------------------- | ------ | ------ |
| Secret scanning (gitleaks) blocks PRs with committed secrets                            | ✅     | 14.3.2 |
| Dependency audit (`cargo-deny` + `pnpm audit`) fails on high/critical CVEs, runs weekly | ✅     | 14.2.x |
| Documented, dated exception process for advisories (`deny.toml`)                        | ✅     | 14.2.x |
| Containers run as non-root; images are minimal (distroless / nginx-unprivileged)        | ✅     | 14.1.x |

## Observability

| Item                                                               | Status | ASVS  |
| ------------------------------------------------------------------ | ------ | ----- |
| Structured JSON logs with a request id (and `x-request-id` header) | ✅     | 7.1.x |
| Authentication events recorded to an audit trail                   | ✅     | 7.2.x |
| Ship logs to a collector and set alerting for your deployment      | 🔶     | 7.3.x |

## When you instantiate this template

Work the 🔶 rows for your case:

1. Set `APP_DOMAIN`, `ACME_EMAIL`, `CORS_ALLOWED_ORIGINS`, and provision unique
   secrets per environment (never reuse the `.env.example` placeholders).
2. Configure real OAuth providers (`OAUTH_*`) and payment keys (`STRIPE_*`) if
   you enable them; supply a real webhook signing secret.
3. Add resource-level authorization and input validation for every new domain
   you introduce, following the existing patterns.
4. Decide on MFA / breached-password checks and PII/encryption-at-rest for your
   data-protection requirements.
5. Point logs at a collector and add alerting.

The 🔶 items are the boundary of what a template can do for you; the ✅ items are
the baseline it will not let you regress (CI enforces the secret scan, the
dependency audit, and the API contract).
