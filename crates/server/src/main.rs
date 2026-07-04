//! Composition root — the only place that knows every layer.
//!
//! Its responsibilities, and nothing else:
//!   1. Load configuration from the environment.
//!   2. Build infrastructure adapters.
//!   3. Inject them into application services and the API router.
//!   4. Start the HTTP server.

use std::sync::Arc;

use application::HealthService;
use infrastructure::AlwaysReady;

mod config;

use config::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Configuration from the environment.
    let config = Config::from_env()?;

    // 2 + 3. Build the infrastructure adapter, inject it into the application
    // service, then hand that to the API layer to build the router.
    let health = HealthService::new(Arc::new(AlwaysReady));
    let app = api::router(health);

    // 4. Serve.
    let listener = tokio::net::TcpListener::bind(config.socket_addr()).await?;
    println!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}
