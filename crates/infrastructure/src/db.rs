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
    /// - `DATABASE_URL` — connection string (required). May instead be supplied
    ///   as a file via `DATABASE_URL_FILE` (the Docker-secrets convention, where
    ///   a secret is mounted under `/run/secrets`); the direct variable wins when
    ///   both are set, and a trailing newline in the file is trimmed.
    /// - `DATABASE_MAX_CONNECTIONS` — pool ceiling (default `10`)
    /// - `DATABASE_MIN_CONNECTIONS` — warm minimum (default `0`)
    /// - `DATABASE_ACQUIRE_TIMEOUT_SECS` — acquire timeout in seconds (default `30`)
    pub fn from_env() -> Result<Self, PgConfigError> {
        let url = resolve_secret(
            "DATABASE_URL",
            std::env::var("DATABASE_URL").ok(),
            std::env::var("DATABASE_URL_FILE").ok(),
        )?
        .filter(|u| !u.trim().is_empty())
        .ok_or(PgConfigError::MissingUrl)?;

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

/// Resolve a secret from a direct value or a `*_FILE` indirection.
///
/// `direct` (the `NAME` variable) takes precedence; otherwise the value is read
/// from the file at `file` (the `NAME_FILE` variable), which is how a Docker
/// secret — mounted as a file under `/run/secrets` — is consumed. A trailing
/// newline (as most secret files carry) is trimmed. Returns `None` when neither
/// is provided; a `NAME_FILE` that cannot be read is a hard error, since a
/// misconfigured secret should fail fast, not fall through to "missing".
///
/// Kept as a pure function (values passed in, not read from the environment) so
/// it is testable without mutating process-global state.
fn resolve_secret(
    name: &str,
    direct: Option<String>,
    file: Option<String>,
) -> Result<Option<String>, PgConfigError> {
    if let Some(value) = direct.filter(|v| !v.trim().is_empty()) {
        return Ok(Some(value));
    }
    if let Some(path) = file.filter(|p| !p.trim().is_empty()) {
        let contents =
            std::fs::read_to_string(&path).map_err(|e| PgConfigError::UnreadableSecretFile {
                key: format!("{name}_FILE"),
                path,
                error: e.to_string(),
            })?;
        return Ok(Some(contents.trim_end_matches(['\n', '\r']).to_string()));
    }
    Ok(None)
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
    /// Neither `DATABASE_URL` nor `DATABASE_URL_FILE` yielded a non-empty value.
    MissingUrl,
    /// A numeric setting was present but could not be parsed.
    InvalidValue { key: String, value: String },
    /// A `*_FILE` secret indirection pointed at a file that could not be read.
    UnreadableSecretFile {
        key: String,
        path: String,
        error: String,
    },
}

impl fmt::Display for PgConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingUrl => write!(
                f,
                "DATABASE_URL is required and must be non-empty (or provide DATABASE_URL_FILE)"
            ),
            Self::InvalidValue { key, value } => write!(f, "invalid {key}: {value:?}"),
            Self::UnreadableSecretFile { key, path, error } => {
                write!(
                    f,
                    "{key} points at {path:?} which could not be read: {error}"
                )
            }
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

    #[test]
    fn resolve_secret_prefers_the_direct_value() {
        let out = resolve_secret(
            "DATABASE_URL",
            Some("postgres://direct".to_string()),
            Some("/does/not/matter".to_string()),
        )
        .unwrap();
        assert_eq!(out.as_deref(), Some("postgres://direct"));
    }

    #[test]
    fn resolve_secret_reads_the_file_and_trims_newline() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("authapp-secret-{}.txt", std::process::id()));
        std::fs::write(&path, "postgres://from-file\n").unwrap();

        let out = resolve_secret(
            "DATABASE_URL",
            None,
            Some(path.to_string_lossy().into_owned()),
        )
        .unwrap();
        assert_eq!(out.as_deref(), Some("postgres://from-file"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn resolve_secret_is_none_when_neither_is_provided() {
        assert!(resolve_secret("DATABASE_URL", None, None)
            .unwrap()
            .is_none());
        // An empty direct value falls through to "none", not to an empty URL.
        assert!(resolve_secret("DATABASE_URL", Some("  ".to_string()), None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn resolve_secret_errors_when_the_file_is_unreadable() {
        let err = resolve_secret(
            "DATABASE_URL",
            None,
            Some("/no/such/secret/file".to_string()),
        )
        .unwrap_err();
        assert!(matches!(err, PgConfigError::UnreadableSecretFile { .. }));
    }
}
