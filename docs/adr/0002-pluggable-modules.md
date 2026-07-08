# 0002 — Pluggable modules registered at the composition root

Status: accepted

## Context

"Modular, sin acoplamiento" has to mean something concrete: adding a feature
must not require editing existing features, and removing one must not break the
others. That needs a seam both the backend and the frontend agree on.

## Decision

A feature is a self-contained unit that the composition root _registers_, and
nothing else references it.

**Backend** — a feature crate implements the `contracts::Module` trait:

```rust
pub trait Module: Send + Sync {
    fn name(&self) -> &'static str;
    fn migrations(&self) -> Vec<Migration> { Vec::new() }
    fn init(&self) -> Result<(), Box<dyn Error + Send + Sync>> { Ok(()) }
    fn router(&self) -> Router { Router::new() }
}
```

It owns its migrations, its initialization, and the routes it mounts. The server
composes them: `ModuleRegistry::new().register(MyModule::new(...))`. Registration
is the _only_ line that mentions the module (see `module-demo` for a worked
example).

**Frontend** — a feature package exports a `FeatureModule` (from
`@auth-app/feature-kit`): the routes it contributes and its sidebar entries. The
shell discovers features through one array in `apps/web/src/shell/registry.ts`
and never names a feature otherwise.

## Consequences

- Adding a feature is: publish a crate/package, then add one registration line.
  No existing module — and no shell/layout code — changes.
- Route and migration ordering/collision are handled by the registry, not by
  each feature.
- The same "register, don't reference" shape holds on both sides of the stack,
  so the mental model transfers. See [`adding-a-module.md`](../adding-a-module.md).
