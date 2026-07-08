# 0004 — Provider-agnostic boundaries (payments, OAuth, database)

Status: accepted

## Context

A template is instantiated against different providers: one team uses Stripe,
another a different processor; one uses Google OIDC, another Okta. If a provider
SDK's types leak into the core, switching provider means rewriting business
logic — the opposite of a reusable template.

## Decision

Every external dependency sits behind a **domain port** (a trait) whose types
are all owned by this codebase; a concrete adapter implements the port in
`infrastructure`, and the composition root selects which adapter to inject.

- **Payments** — the `PaymentProvider` port speaks only in the payments crate's
  own types (`Money`, `PaymentStatus`, `ProviderReference`). `StripeProvider` and
  a deterministic `FakePaymentProvider` implement it; `PAYMENT_PROVIDER` selects
  one at startup. No Stripe SDK type crosses the port.
- **OAuth/OIDC** — one generic `OidcProvider` adapter serves every configured
  provider off shared config (`OAUTH_<X>_*`); adding a provider is configuration,
  not code.
- **Database** — repositories are domain ports; the Postgres adapters live in
  `infrastructure`. The application layer holds `Arc<dyn SomeRepository>`, never a
  `sqlx` type.

## Consequences

- Switching or adding a provider changes _one adapter and one config value_ —
  never the domain, application, or API layers.
- The whole flow is testable without the real provider: the fake payment
  provider and a scripted HTTP transport exercise the logic deterministically,
  so integration tests need no credentials or network to the provider.
- The honest limitation: the real socket to an external provider (Stripe's API,
  a live OIDC endpoint) is only exercised in a real deployment — tests verify our
  side of the contract, not the provider's.
