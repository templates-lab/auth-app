# Adding a module

How to add a feature — a backend crate and/or a frontend package — without
touching any existing module. This is the payoff of
[ADR 0002](adr/0002-pluggable-modules.md): you _register_ a feature; nothing
references it.

The worked examples in the repo are `crates/module-demo` (backend) and
`packages/feature-dashboard` / `packages/feature-transactions` (frontend).

## Backend: a feature crate

1. **Create the crate** under `crates/` (the workspace picks up `crates/*`
   automatically):

   ```
   crates/feature-widgets/
   ├── Cargo.toml
   └── src/lib.rs
   ```

   `Cargo.toml` depends only on `contracts` (plus whatever the feature needs):

   ```toml
   [package]
   name = "feature-widgets"
   edition.workspace = true
   rust-version.workspace = true

   [lints]
   workspace = true

   [dependencies]
   contracts.workspace = true
   axum.workspace = true
   ```

2. **Implement `contracts::Module`.** Own your routes, migrations, and init:

   ```rust
   use axum::{routing::get, Router};
   use contracts::{Migration, Module};

   #[derive(Debug, Clone)]
   pub struct Widgets;

   impl Module for Widgets {
       fn name(&self) -> &'static str { "widgets" }

       fn migrations(&self) -> Vec<Migration> {
           vec![Migration::new("0001_widgets", "CREATE TABLE widgets (id UUID PRIMARY KEY);")]
       }

       fn router(&self) -> Router {
           Router::new().route("/widgets", get(|| async { "ok" }))
       }
   }
   ```

3. **Register it at the composition root** — the only line that mentions the
   module, in `crates/server/src/main.rs`:

   ```rust
   let modules = ModuleRegistry::new()
       .register(module_demo::Demo::new("…"))
       .register(feature_widgets::Widgets); // ← added
   ```

   Add `feature-widgets` to `crates/server/Cargo.toml`'s `[dependencies]`. The
   registry runs your migrations, calls `init()`, and mounts `router()` — and
   rejects a duplicate module name.

4. **Verify:** `cargo build -p feature-widgets && cargo build -p server`.

For real business logic, keep the hexagonal split ([ADR 0001](adr/0001-hexagonal-architecture.md)):
put the model + ports in a `domain`-style crate, the use cases in `application`,
and the adapters in `infrastructure` — the module crate wires them.

## Frontend: a feature package

1. **Create the package** under `packages/` (the workspace picks up `packages/*`):

   ```
   packages/feature-widgets/
   ├── package.json     # name "@auth-app/feature-widgets", exports ./src/index.tsx
   ├── tsconfig.json    # extends ../../tsconfig.base.json (jsx: preserve, solid-js)
   └── src/index.tsx
   ```

   Copy `packages/feature-users/{package.json,tsconfig.json}` and rename — they
   are the minimal, correct shape (ships Solid source; peer-deps on `solid-js`
   and `@solidjs/router`).

2. **Export a `FeatureModule`** with `defineFeature` from `@auth-app/feature-kit`:

   ```tsx
   import { lazy } from "solid-js";
   import { defineFeature } from "@auth-app/feature-kit";

   export const widgetsFeature = defineFeature({
     id: "widgets",
     title: "Widgets",
     nav: [{ path: "/widgets", label: "Widgets", icon: "▤", order: 40 }],
     routes: [
       {
         path: "/widgets",
         component: lazy(() => import("./Widgets").then((m) => ({ default: m.Widgets }))),
       },
     ],
   });
   ```

   Data-fetching uses `@auth-app/query` (typed query keys, the api-client bridge);
   see `packages/feature-transactions` for a feature that lists, shows detail, and
   mutates through TanStack Query.

3. **Register it in the shell** — the only place a feature is wired in,
   `apps/web/src/shell/registry.ts`:

   ```ts
   import { widgetsFeature } from "@auth-app/feature-widgets";
   export const features: FeatureModule[] = [
     dashboardFeature,
     usersFeature,
     transactionsFeature,
     widgetsFeature,
   ];
   ```

   Add `@auth-app/feature-widgets` to `apps/web/package.json` dependencies, and —
   because it ships `.tsx` source — to `optimizeDeps.exclude` in
   `apps/web/vite.config.ts` (next to the other feature packages).

4. **Verify:** `pnpm install && pnpm -r build`. The sidebar entry and route
   appear with no change to any other feature or to the layout.

## What you did _not_ touch

No existing crate, package, feature, layout, or router file changed — only the
two registration points (`server/main.rs`, `shell/registry.ts`) and the two
manifests that declare the new dependency. That is the invariant the module
system guarantees.
