//! Postgres integration: an env-configured connection pool, versioned
//! migrations embedded at compile time, and a startup schema check.
//!
//! This module owns everything that touches the database driver. The pool it
//! builds is injected into adapters (e.g. [`crate::PgHealthCheck`]) by the
//! composition root; the domain and application layers never see it.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use sqlx::migrate::{MigrateError, Migrator};
use sqlx::postgres::{PgPool, PgPoolOptions};

/// The versioned SQL migrations, embedded at compile time from
/// `crates/infrastructure/migrations`.
///
/// New modules drop their `NNNN_*.sql` files in that directory and they are
/// picked up automatically — no code change here. Embedding at compile time
/// means the binary carries its own schema and needs no migration files on the
/// deployment host.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Connection-pool configuration, sourced entirely from the environment.
#[derive(Debug, Clone)]
pub struct PgConfig {
    /// sqlx/libpq connection string, e.g. `postgres://user:pass@host/db`.
    pub url: String,
    /// Upper bound on the number of pooled connections.
    pub max_connections: u32,
    /// Connections kept open even while idle.
    pub min_connections: u32,
    /// How long `acquire` waits for a free connection before erroring.
    pub acquire_timeout: Duration,
}

impl PgConfig {
    /// Read pool configuration from the environment.
    ///
    /// - `DATABASE_URL` — connection string (required)
    /// - `DATABASE_MAX_CONNECTIONS` — pool ceiling (default `10`)
    /// - `DATABASE_MIN_CONNECTIONS` — warm minimum (default `0`)
    /// - `DATABASE_ACQUIRE_TIMEOUT_SECS` — acquire timeout in seconds (default `30`)
    pub fn from_env() -> Result<Self, PgConfigError> {
        let url = std::env::var("DATABASE_URL").map_err(|_| PgConfigError::MissingUrl)?;
        if url.trim().is_empty() {
            return Err(PgConfigError::MissingUrl);
        }

        let max_connections = parse_env("DATABASE_MAX_CONNECTIONS", 10)?;
        let min_connections = parse_env("DATABASE_MIN_CONNECTIONS", 0)?;
        let acquire_secs = parse_env::<u64>("DATABASE_ACQUIRE_TIMEOUT_SECS", 30)?;

        Ok(Self {
            url,
            max_connections,
            min_connections,
            acquire_timeout: Duration::from_secs(acquire_secs),
        })
    }
}

/// Parse an optional environment variable, falling back to `default` when unset.
///
/// A present-but-unparseable value is an error rather than a silent fallback, so
/// a typo in `DATABASE_MAX_CONNECTIONS` fails fast at startup.
fn parse_env<T>(key: &str, default: T) -> Result<T, PgConfigError>
where
    T: FromStr,
{
    match std::env::var(key) {
        Ok(raw) => raw.parse().map_err(|_| PgConfigError::InvalidValue {
            key: key.to_string(),
            value: raw,
        }),
        Err(_) => Ok(default),
    }
}

/// Build and eagerly connect a Postgres pool from configuration.
///
/// The pool opens a connection immediately, so a misconfigured or unreachable
/// database is caught here at startup rather than on the first request.
pub async fn connect(config: &PgConfig) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(config.acquire_timeout)
        .connect(&config.url)
        .await
}

/// Apply pending migrations and verify the recorded schema at startup.
///
/// [`Migrator::run`] applies any not-yet-applied migrations and validates the
/// checksums of the ones already recorded in `_sqlx_migrations`. A database
/// whose applied schema has drifted from the embedded migrations therefore
/// fails fast here instead of misbehaving later.
pub async fn run_migrations(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(pool).await
}

/// A malformed or missing database environment value.
#[derive(Debug)]
pub enum PgConfigError {
    /// `DATABASE_URL` was unset or empty.
    MissingUrl,
    /// A numeric setting was present but could not be parsed.
    InvalidValue { key: String, value: String },
}

impl fmt::Display for PgConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingUrl => write!(f, "DATABASE_URL is required and must be non-empty"),
            Self::InvalidValue { key, value } => write!(f, "invalid {key}: {value:?}"),
        }
    }
}

impl std::error::Error for PgConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded migrator must carry at least the baseline migration, and its
    /// versions must be strictly increasing (a duplicate or out-of-order file
    /// would break `sqlx migrate run` from scratch).
    #[test]
    fn migrations_are_embedded_and_ordered() {
        let versions: Vec<i64> = MIGRATOR.iter().map(|m| m.version).collect();
        assert!(
            !versions.is_empty(),
            "expected at least the baseline migration"
        );

        let mut sorted = versions.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            versions, sorted,
            "migration versions must be unique and ascending"
        );
    }
}
