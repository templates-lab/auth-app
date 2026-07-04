//! Composition root — the only place that knows every layer.
//!
//! Its responsibilities, and nothing else:
//!   1. Load configuration from the environment.
//!   2. Register the feature modules.
//!   3. Run their migrations, then initialize them.
//!   4. Build the base router and let each module mount its routes.
//!   5. Start the HTTP server.

use std::sync::Arc;

use application::HealthService;
use contracts::{InMemoryExecutor, ModuleRegistry};
use infrastructure::AlwaysReady;

mod config;

use config::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Configuration from the environment.
    let config = Config::from_env()?;

    // 2. Register the feature modules. Adding a feature is ONE line here; its
    // crate carries everything else (routes, migrations, init). Removing a
    // feature is the reverse and never touches another module.
    let modules =
        ModuleRegistry::new().register(module_demo::Demo::new("hello from the demo module"));

    // 3. Apply every module's migrations — isolated per module and ordered —
    // before serving. `InMemoryExecutor` is the seam a database-backed executor
    // replaces once the infrastructure layer grows a driver.
    let mut migrator = InMemoryExecutor::default();
    let report = modules.run_migrations(&mut migrator)?;
    println!(
        "migrations: {} applied, {} already present",
        report.applied.len(),
        report.skipped.len()
    );

    // Initialize the modules once their schema is in place.
    modules.init()?;

    // 4. Build the base router (the core health probe), then let each module
    // merge its own routes onto it.
    let health = HealthService::new(Arc::new(AlwaysReady));
    let app = modules.router(api::router(health));

    // 5. Serve.
    let listener = tokio::net::TcpListener::bind(config.socket_addr()).await?;
    println!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}
