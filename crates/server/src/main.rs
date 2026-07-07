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
    AuditService, BootstrapOutcome, BootstrapService, HealthService, LoginService,
    OAuthLoginService, SessionService, WebhookService,
};
use contracts::{InMemoryExecutor, ModuleRegistry};
use infrastructure::{
    Argon2Hasher, FakePaymentProvider, OAuthSecrets, OidcProvider, PgAdminRepository,
    PgAuditRepository, PgConfig, PgHealthCheck, PgIpLockoutStore, PgOAuthIdentityRepository,
    PgPaymentRepository, PgPendingAuthStore, PgSessionRepository, PgWebhookEventStore,
    ReqwestHttpClient, SecureRandomTokens, StripeProvider, StripeWebhookConfig,
    StripeWebhookVerifier, SystemClock,
};

mod config;

use config::{AuthConfig, Config, OAuthSettings, PaymentProviderConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("serve") | None => serve().await,
        Some("bootstrap-admin") => bootstrap_admin().await,
        // Print the OpenAPI spec to stdout and exit. Needs no database; the
        // monorepo's `gen:api` script pipes this into the TypeScript client
        // generator.
        Some("openapi") => {
            println!("{}", api::ApiDoc::to_pretty_json());
            Ok(())
        }
        Some(other) => {
            eprintln!(
                "server: unknown command {other:?}\n\n\
                 Commands:\n  \
                 serve            Run the HTTP server (default)\n  \
                 bootstrap-admin  Create the first admin from \
                 ADMIN_BOOTSTRAP_EMAIL / ADMIN_BOOTSTRAP_PASSWORD\n  \
                 openapi          Print the OpenAPI spec (for client generation)"
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

    // OAuth is optional: enabled only when providers are configured. The
    // generic OIDC adapter serves every configured provider off one shared
    // HTTP client — adding a provider is configuration, not code.
    let oauth = OAuthSettings::from_env()?.map(|settings| {
        let http = Arc::new(ReqwestHttpClient::new());
        let providers = settings
            .providers
            .into_iter()
            .map(|cfg| {
                Arc::new(OidcProvider::new(cfg, http.clone())) as Arc<dyn domain::OAuthProvider>
            })
            .collect();
        let service = OAuthLoginService::new(
            providers,
            Arc::new(PgPendingAuthStore::new(pool.clone())),
            Arc::new(PgOAuthIdentityRepository::new(pool.clone())),
            Arc::new(PgAdminRepository::new(pool.clone())),
            Arc::new(OAuthSecrets),
            Arc::new(SystemClock),
            settings.redirect_base,
        );
        (service, settings.redirects)
    });

    // Select the active payment provider by env (fake for local dev, stripe in
    // production) — no domain/application logic recompiles when it changes. The
    // payment webhook endpoint is enabled for Stripe once its signing secret is
    // set; the provider handle itself feeds the admin transactions API in a
    // follow-up bead. Building both here proves the env selection and validates
    // the configuration at startup.
    let payment_config = PaymentProviderConfig::from_env()?;
    println!("payments: provider = {}", payment_config.label());
    #[allow(clippy::type_complexity)]
    let (_payment_provider, webhooks): (
        Option<Arc<dyn payments::PaymentProvider>>,
        Option<WebhookService>,
    ) = match payment_config {
        PaymentProviderConfig::Disabled => (None, None),
        PaymentProviderConfig::Fake => (Some(Arc::new(FakePaymentProvider::new())), None),
        PaymentProviderConfig::Stripe(cfg) => {
            let provider: Arc<dyn payments::PaymentProvider> =
                Arc::new(StripeProvider::new(cfg, Arc::new(ReqwestHttpClient::new())));
            let webhooks = std::env::var("STRIPE_WEBHOOK_SECRET")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(|secret| {
                    println!("payments: stripe webhooks enabled");
                    WebhookService::new(
                        Arc::new(StripeWebhookVerifier::new(StripeWebhookConfig::new(secret))),
                        Arc::new(PgWebhookEventStore::new(pool.clone())),
                        Arc::new(PgPaymentRepository::new(pool.clone())),
                    )
                });
            (Some(provider), webhooks)
        }
    };

    let app = modules.router(api::router(
        health,
        login,
        sessions,
        audit,
        oauth,
        webhooks,
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
