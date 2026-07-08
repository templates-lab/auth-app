# 0001 — Hexagonal architecture with a one-way crate graph

Status: accepted

## Context

The template must stay modular and un-coupled as features accrete, and it must
keep business logic testable without a database or a web server. A conventional
layered app tends to let framework and storage types leak into the core, which
makes the core hard to test and change.

## Decision

The backend is a Cargo workspace split into crates whose dependencies point in
one direction only:

```
domain  ←  application  ←  api  ←  server
   ↑            ↑                    │
   └── infrastructure ───────────────┘ (implements domain ports; injected at the root)
```

- **domain** — business model and _ports_ (traits). Depends on nothing but the
  standard library (plus small ergonomics macros). No `axum`, no `sqlx`.
- **application** — use cases orchestrating the domain through its ports. Depends
  only on `domain`.
- **infrastructure** — adapters that _implement_ the domain ports (Postgres,
  argon2, OIDC HTTP, Stripe). Depends on `domain`; nothing depends _on it_ except
  the composition root.
- **api** — the HTTP boundary (axum). Translates requests to application calls.
- **server** — the composition root: reads config, builds adapters, injects them
  into services, mounts the router. The _only_ crate that knows every layer.

`payments` is a second, independent domain crate (a bounded context) with the
same discipline.

## Consequences

- The domain and application layers are unit-tested with in-memory fakes — no
  Docker, no network. Postgres-backed integration tests live in the adapter and
  api crates.
- Swapping an adapter (a different payment provider, a different database) never
  touches the domain or application layers — only the composition root picks a
  different implementation of the same port.
- The dependency direction is enforced structurally: a crate simply cannot
  `use` something from a crate it does not depend on, so the boundary cannot rot
  silently.
