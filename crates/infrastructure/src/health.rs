//! Database-backed health adapter.

use async_trait::async_trait;
use domain::{Health, HealthCheck, Readiness};
use sqlx::postgres::PgPool;

/// A [`HealthCheck`] adapter backed by a live Postgres pool.
///
/// Readiness is decided by executing a trivial `SELECT 1`: if it succeeds the
/// service is [`Readiness::Ready`]; if it fails for any reason — the database is
/// down, the pool is exhausted, the acquire times out — the service reports
/// [`Readiness::NotReady`] so callers (the `/health` probe) can shed traffic.
#[derive(Debug, Clone)]
pub struct PgHealthCheck {
    pool: PgPool,
}

impl PgHealthCheck {
    /// Build the adapter over an existing pool (shared with the rest of the app).
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl HealthCheck for PgHealthCheck {
    async fn check(&self) -> Health {
        let readiness = match sqlx::query("SELECT 1").execute(&self.pool).await {
            Ok(_) => Readiness::Ready,
            Err(_) => Readiness::NotReady,
        };
        Health { readiness }
    }
}
