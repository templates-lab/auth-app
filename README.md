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
│   ├── domain/          Business model and ports — no framework deps
│   ├── application/     Use cases orchestrating the domain
│   ├── infrastructure/  Adapters implementing domain ports
│   ├── api/             HTTP boundary (axum router)
│   ├── server/          Composition root — the `server` binary
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
