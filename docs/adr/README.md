# Architecture Decision Records

Short, immutable records of the decisions that shape this template — the
_why_ behind the structure, so someone extending it can tell load-bearing
choices from incidental ones.

Each ADR states the context, the decision, and its consequences. They are
append-only: a superseded decision gets a new ADR that references the old one,
rather than an edit.

| #                                            | Decision                                             |
| -------------------------------------------- | ---------------------------------------------------- |
| [0001](0001-hexagonal-architecture.md)       | Hexagonal architecture with a one-way crate graph    |
| [0002](0002-pluggable-modules.md)            | Pluggable modules registered at the composition root |
| [0003](0003-server-sessions-over-jwt.md)     | Server-side sessions (cookies), not JWTs             |
| [0004](0004-provider-agnostic-boundaries.md) | Provider-agnostic boundaries (payments, OAuth, DB)   |

See also [`adding-a-module.md`](../adding-a-module.md) for the step-by-step that
these decisions make possible.
