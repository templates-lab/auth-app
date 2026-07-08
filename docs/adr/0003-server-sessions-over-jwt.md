# 0003 — Server-side sessions (cookies), not JWTs

Status: accepted

## Context

The admin app needs authenticated sessions. The two common choices are a
stateless signed token (JWT) held by the client, or a server-side session
referenced by an opaque cookie.

## Decision

Authentication uses **server-side sessions**. On login the server stores a
session row and sets two cookies:

- `session` — an opaque bearer token, `HttpOnly` + `Secure` +
  `SameSite=Strict`, so client-side script cannot read it and it is not sent
  cross-site.
- `csrf` — a readable token the client mirrors into an `X-CSRF-Token` header on
  every mutating request; the server checks it. `GET`/`HEAD`/`OPTIONS` are exempt.

Sessions have idle and absolute timeouts and are validated against storage on
every request. Logout and expiry revoke server-side.

## Consequences

- **Revocation is immediate.** Logout, a password change, or an admin action can
  invalidate a session instantly — a stateless JWT cannot be revoked before it
  expires without reintroducing server state.
- **No token handling in the browser.** The `HttpOnly` cookie is immune to XSS
  token theft; the SPA never stores or forwards a bearer token.
- **CSRF is handled explicitly** because cookies are sent automatically: the
  double-submit `csrf` cookie + header pair covers mutations.
- Cost: a storage read per authenticated request (a session lookup). Acceptable
  for an admin tool, and it is what makes revocation and idle-timeout real.
- OAuth sign-in ends the same way — it issues an ordinary server session — so
  there is a single session model regardless of how the user authenticated.
