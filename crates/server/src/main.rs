//! Composition root — the only place that knows every layer.
//!
//! The binary has two commands, dispatched from the first argument:
//!
//!   * `serve` (the default when no argument is given) — boot the HTTP server.
//!   * `bootstrap-admin` — create the first administrator from environment
//!     secrets, so no password is ever committed to the repository.
//!
//! The `serve` path's responsibilities, and nothing else:
//!   1. Load configuration from the environment.
//!   2. Register the feature modules.
//!   3. Connect the database pool and run migrations (startup schema check),
//!      then apply each module's own migrations.
//!   4. Initialize the modules.
//!   5. Build the base router with a database-backed health probe and admin
//!      login, and let each module mount its routes.
//!   6. Start the HTTP server.

use std::net::SocketAddr;
use std::sync::Arc;

use application::{
    AuditService, BootstrapOutcome, BootstrapService, HealthService, LoginService, SessionService,
};
use contracts::{InMemoryExecutor, ModuleRegistry};
use infrastructure::{
    Argon2Hasher, PgAdminRepository, PgAuditRepository, PgConfig, PgHealthCheck, PgIpLockoutStore,
    PgSessionRepository, SecureRandomTokens, SystemClock,
};

mod config;

use config::{AuthConfig, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("serve") | None => serve().await,
        Some("bootstrap-admin") => bootstrap_admin().await,
        Some(other) => {
            eprintln!(
                "server: unknown command {other:?}\n\n\
                 Commands:\n  \
                 serve            Run the HTTP server (default)\n  \
                 bootstrap-admin  Create the first admin from \
                 ADMIN_BOOTSTRAP_EMAIL / ADMIN_BOOTSTRAP_PASSWORD"
            );
            Err("unknown command".into())
        }
    }
}

/// Run the HTTP server (the default command).
async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Configuration from the environment.
    let config = Config::from_env()?;
    let auth = AuthConfig::from_env()?;
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

    // 5. Build the base router. The health probe reflects live database
    // connectivity; the login service verifies admin credentials with argon2id,
    // constant-time equalization, and progressive account/IP lockout.
    let health = HealthService::new(Arc::new(PgHealthCheck::new(pool.clone())));
    let login = LoginService::new(
        Arc::new(PgAdminRepository::new(pool.clone())),
        Arc::new(PgIpLockoutStore::new(pool.clone())),
        Arc::new(Argon2Hasher::new(auth.argon2)?),
        Arc::new(SystemClock),
        auth.lockout_policy,
    );
    let sessions = SessionService::new(
        Arc::new(PgSessionRepository::new(pool.clone())),
        Arc::new(SecureRandomTokens),
        Arc::new(SystemClock),
        auth.session_policy,
    );
    let audit = AuditService::new(Arc::new(PgAuditRepository::new(pool.clone())));
    let app = modules.router(api::router(
        health,
        login,
        sessions,
        audit,
        config.cors_allowed_origins(),
        auth.login_rate_limit,
    ));

    // 6. Serve. `into_make_service_with_connect_info` records each connection's
    // peer address so the login handler can fall back to it for per-IP lockout
    // when no forwarded header is present.
    let listener = tokio::net::TcpListener::bind(config.socket_addr()).await?;
    println!("listening on http://{}", listener.local_addr()?);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// Create the first administrator from environment secrets, then exit.
///
/// The email and password come from `ADMIN_BOOTSTRAP_EMAIL` and
/// `ADMIN_BOOTSTRAP_PASSWORD` — secrets injected at run time, never checked into
/// the repository. The operation is idempotent-friendly: it refuses to run once
/// any admin exists, so re-running it cannot reset an account.
async fn bootstrap_admin() -> Result<(), Box<dyn std::error::Error>> {
    let auth = AuthConfig::from_env()?;
    let pg_config = PgConfig::from_env()?;

    let email = std::env::var("ADMIN_BOOTSTRAP_EMAIL")
        .map_err(|_| "ADMIN_BOOTSTRAP_EMAIL is required for bootstrap-admin")?;
    let password = std::env::var("ADMIN_BOOTSTRAP_PASSWORD")
        .map_err(|_| "ADMIN_BOOTSTRAP_PASSWORD is required for bootstrap-admin")?;

    // Ensure the schema exists before we touch it — bootstrap may run against a
    // brand-new database, before the server has ever served.
    let pool = infrastructure::connect(&pg_config).await?;
    infrastructure::run_migrations(&pool).await?;

    let hasher = Arc::new(Argon2Hasher::new(auth.argon2)?);
    let repo = Arc::new(PgAdminRepository::new(pool));
    let bootstrap = BootstrapService::new(repo, hasher, auth.password_policy);

    match bootstrap.create_first_admin(&email, &password).await? {
        BootstrapOutcome::Created(id) => {
            println!("created first admin {email} (id {id})");
        }
        BootstrapOutcome::AlreadyInitialized => {
            println!("an administrator already exists; nothing to do");
        }
    }
    Ok(())
}
