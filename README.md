# auth-app

Authentication app template — a modular monorepo with a Rust backend and a
web frontend, deployable behind Traefik with Postgres.

## Layout

```
.
├── apps/          Frontend applications (pnpm workspace)
│   └── web/       Admin web app (SolidJS + TanStack Query)
├── crates/        Rust backend (Cargo workspace, hexagonal architecture)
│   └── xtask/     Workspace automation (`cargo xtask`)
├── infra/         Deployment
│   ├── docker/    Dockerfiles
│   └── traefik/   Traefik routing and TLS
└── docs/          Architecture and operations docs
```

The Rust crates form a Cargo workspace rooted at `Cargo.toml`; the frontend
apps form a separate pnpm workspace under `apps/`. The two toolchains stay
decoupled on purpose so a change on one side never forces a rebuild of the
other.

## Getting started

```sh
cargo build          # build the Rust workspace
cargo clippy         # lint
cargo fmt            # format (rules in rustfmt.toml)
cargo xtask help     # list workspace tasks
```

Shared Rust conventions live at the root: compilation profiles and lints in
`Cargo.toml`, formatting in `rustfmt.toml`, and Clippy config in `clippy.toml`.
