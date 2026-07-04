//! Composition root — the only place that knows every layer.
//!
//! Its responsibilities, and nothing else:
//!   1. Load configuration from the environment.
//!   2. Register the feature modules.
//!   3. Connect the database pool and run migrations (startup schema check),
//!      then apply each module's own migrations.
//!   4. Initialize the modules.
//!   5. Build the base router with a database-backed health probe and let each
//!      module mount its routes.
//!   6. Start the HTTP server.

use std::sync::Arc;

use application::HealthService;
use contracts::{InMemoryExecutor, ModuleRegistry};
use infrastructure::{PgConfig, PgHealthCheck};

mod config;

use config::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Configuration from the environment.
    let config = Config::from_env()?;
    let pg_config = PgConfig::from_env()?;

    // 2. Register the feature modules. Adding a feature is ONE line here; its
    // crate carries everything else (routes, migrations, init). Removing a
    // feature is the reverse and never touches another module.
    let modules =
        ModuleRegistry::new().register(module_demo::Demo::new("hello from the demo module"));

    // 3. Connect the pool (eagerly, so a bad DSN or down database fails now) and
    // bring the schema up to date. `run_migrations` also validates the checksums
    // of already-applied migrations, so schema drift is caught at startup.
    let pool = infrastructure::connect(&pg_config).await?;
    infrastructure::run_migrations(&pool).await?;

    // Apply every module's migrations — isolated per module and ordered — over
    // the in-memory executor seam a database-backed executor will later fill.
    let mut migrator = InMemoryExecutor::default();
    let report = modules.run_migrations(&mut migrator)?;
    println!(
        "migrations: {} applied, {} already present",
        report.applied.len(),
        report.skipped.len()
    );

    // 4. Initialize the modules once their schema is in place.
    modules.init()?;

    // 5. Build the base router with a database-backed health probe, then let each
    // module merge its own routes onto it. The `/health` probe reflects live
    // database connectivity, so readiness fails when the database is unavailable.
    let health = HealthService::new(Arc::new(PgHealthCheck::new(pool)));
    let app = modules.router(api::router(health));

    // 6. Serve.
    let listener = tokio::net::TcpListener::bind(config.socket_addr()).await?;
    println!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}
