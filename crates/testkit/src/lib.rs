//! Shared integration-test harness.
//!
//! [`spawn_test_db`] starts a fresh, ephemeral Postgres container via
//! `testcontainers`, applies every one of `infrastructure`'s embedded
//! migrations against it, and hands back a connected pool. Call it once per
//! test: each call gets its own container, so tests run on a schema no other
//! test has ever touched — no manual `docker run`, no shared database, no
//! cross-test bleed. Requires a running Docker daemon.

use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;

/// A live ephemeral Postgres instance backing one test.
///
/// Keep this bound for the duration of the test (e.g. `let db =
/// spawn_test_db().await;`) — dropping it stops and removes the container.
/// Binding it to `_` drops it immediately and the pool stops working.
pub struct TestDb {
    _container: ContainerAsync<Postgres>,
    /// A pool connected to the fresh, migrated database.
    pub pool: PgPool,
}

impl std::fmt::Debug for TestDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestDb").finish_non_exhaustive()
    }
}

/// Start a fresh Postgres container, migrate it, and connect a pool to it.
///
/// # Panics
///
/// Panics if Docker is unavailable, the container fails to start, or the
/// connection/migration step fails — any of which means the test environment
/// itself is broken, not the code under test, so failing loudly and early is
/// preferable to a confusing downstream assertion failure.
pub async fn spawn_test_db() -> TestDb {
    let container = Postgres::default()
        .start()
        .await
        .expect("failed to start ephemeral postgres container — is Docker running?");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to read the mapped postgres port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let pool = PgPool::connect(&url)
        .await
        .expect("failed to connect to the ephemeral postgres container");
    infrastructure::run_migrations(&pool)
        .await
        .expect("failed to run migrations against the ephemeral postgres container");

    TestDb {
        _container: container,
        pool,
    }
}
