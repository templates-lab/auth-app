//! Server configuration, sourced entirely from the environment.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

use domain::{LockoutPolicy, PasswordPolicy, SessionPolicy};
use infrastructure::Argon2Params;

/// Runtime configuration for the HTTP server.
#[derive(Debug, Clone)]
pub(crate) struct Config {
    host: IpAddr,
    port: u16,
    cors_allowed_origins: Vec<String>,
}

impl Config {
    /// Read configuration from the environment.
    ///
    /// - `APP_HOST` — bind address (default `0.0.0.0`)
    /// - `APP_PORT` — bind port (default `8080`)
    /// - `CORS_ALLOWED_ORIGINS` — comma-separated exact origins allowed to make
    ///   credentialed cross-origin requests (e.g.
    ///   `https://admin.example.com,http://localhost:5173`). Unset or empty
    ///   means no origin is allowed — there is no wildcard fallback; a
    ///   production deployment behind a single origin (web + API same host,
    ///   the default Traefik routing) needs nothing here at all.
    pub(crate) fn from_env() -> Result<Self, ConfigError> {
        let host = match std::env::var("APP_HOST") {
            Ok(raw) => raw.parse().map_err(|_| ConfigError::InvalidHost(raw))?,
            Err(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        let port = match std::env::var("APP_PORT") {
            Ok(raw) => raw.parse().map_err(|_| ConfigError::InvalidPort(raw))?,
            Err(_) => 8080,
        };
        let cors_allowed_origins = std::env::var("CORS_ALLOWED_ORIGINS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|origin| !origin.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            host,
            port,
            cors_allowed_origins,
        })
    }

    /// The address the HTTP server should bind to.
    pub(crate) fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }

    /// The explicit CORS origin allowlist (possibly empty — never a wildcard).
    pub(crate) fn cors_allowed_origins(&self) -> &[String] {
        &self.cors_allowed_origins
    }
}

/// A malformed environment value.
#[derive(Debug)]
pub(crate) enum ConfigError {
    InvalidHost(String),
    InvalidPort(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHost(v) => write!(f, "invalid APP_HOST: {v:?}"),
            Self::InvalidPort(v) => write!(f, "invalid APP_PORT: {v:?}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Authentication configuration: the password policy, lockout policy, and argon2
/// cost parameters, all sourced from the environment so an operator tunes them
/// without a rebuild. The secrets themselves (the bootstrap password) are read
/// separately at the point of use, never held here.
#[derive(Debug, Clone)]
pub(crate) struct AuthConfig {
    /// Argon2id cost parameters (OWASP defaults).
    pub(crate) argon2: Argon2Params,
    /// The password-strength policy enforced at bootstrap.
    pub(crate) password_policy: PasswordPolicy,
    /// The progressive lockout policy applied on failed logins.
    pub(crate) lockout_policy: LockoutPolicy,
    /// The session idle/absolute expiration policy.
    pub(crate) session_policy: SessionPolicy,
}

impl AuthConfig {
    /// Read the auth configuration from the environment.
    ///
    /// Password policy:
    /// - `ADMIN_PASSWORD_MIN_LENGTH` (default `12`)
    /// - `ADMIN_PASSWORD_REQUIRE_UPPERCASE` / `_LOWERCASE` / `_DIGIT` (default `true`)
    /// - `ADMIN_PASSWORD_REQUIRE_SYMBOL` (default `false`)
    ///
    /// Lockout policy:
    /// - `ADMIN_LOCKOUT_MAX_ATTEMPTS` (default `5`)
    /// - `ADMIN_LOCKOUT_BASE_SECONDS` (default `60`)
    /// - `ADMIN_LOCKOUT_MAX_SECONDS` (default `3600`)
    ///
    /// Session policy:
    /// - `SESSION_IDLE_TIMEOUT_SECS` (default `1800`, 30 minutes)
    /// - `SESSION_ABSOLUTE_TIMEOUT_SECS` (default `43200`, 12 hours)
    ///
    /// Argon2 parameters follow [`Argon2Params::from_env`] (OWASP defaults). A
    /// present-but-unparseable value is an error, never a silent fallback.
    pub(crate) fn from_env() -> Result<Self, AuthConfigError> {
        let recommended = PasswordPolicy::recommended();
        let password_policy = PasswordPolicy {
            min_length: parse_var("ADMIN_PASSWORD_MIN_LENGTH", recommended.min_length)?,
            require_uppercase: parse_bool(
                "ADMIN_PASSWORD_REQUIRE_UPPERCASE",
                recommended.require_uppercase,
            )?,
            require_lowercase: parse_bool(
                "ADMIN_PASSWORD_REQUIRE_LOWERCASE",
                recommended.require_lowercase,
            )?,
            require_digit: parse_bool("ADMIN_PASSWORD_REQUIRE_DIGIT", recommended.require_digit)?,
            require_symbol: parse_bool(
                "ADMIN_PASSWORD_REQUIRE_SYMBOL",
                recommended.require_symbol,
            )?,
        };

        let lockout = LockoutPolicy::recommended();
        let lockout_policy = LockoutPolicy {
            max_attempts: parse_var("ADMIN_LOCKOUT_MAX_ATTEMPTS", lockout.max_attempts)?,
            base_delay: Duration::from_secs(parse_var(
                "ADMIN_LOCKOUT_BASE_SECONDS",
                lockout.base_delay.as_secs(),
            )?),
            max_delay: Duration::from_secs(parse_var(
                "ADMIN_LOCKOUT_MAX_SECONDS",
                lockout.max_delay.as_secs(),
            )?),
        };

        let argon2 = Argon2Params::from_env()
            .map_err(|e| AuthConfigError::InvalidValue("ARGON2_*".to_string(), e.to_string()))?;

        let recommended_session = SessionPolicy::recommended();
        let session_policy = SessionPolicy {
            idle_timeout: Duration::from_secs(parse_var(
                "SESSION_IDLE_TIMEOUT_SECS",
                recommended_session.idle_timeout.as_secs(),
            )?),
            absolute_timeout: Duration::from_secs(parse_var(
                "SESSION_ABSOLUTE_TIMEOUT_SECS",
                recommended_session.absolute_timeout.as_secs(),
            )?),
        };

        Ok(Self {
            argon2,
            password_policy,
            lockout_policy,
            session_policy,
        })
    }
}

/// Parse an optional environment variable, falling back to `default` when unset.
fn parse_var<T>(key: &str, default: T) -> Result<T, AuthConfigError>
where
    T: FromStr,
{
    match std::env::var(key) {
        Ok(raw) => raw
            .parse()
            .map_err(|_| AuthConfigError::InvalidValue(key.to_string(), raw)),
        Err(_) => Ok(default),
    }
}

/// Parse a boolean environment variable, accepting `true/false/1/0/yes/no`.
fn parse_bool(key: &str, default: bool) -> Result<bool, AuthConfigError> {
    match std::env::var(key) {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(AuthConfigError::InvalidValue(key.to_string(), raw)),
        },
        Err(_) => Ok(default),
    }
}

/// A malformed authentication configuration value.
#[derive(Debug)]
pub(crate) enum AuthConfigError {
    /// A setting was present but could not be parsed.
    InvalidValue(String, String),
}

impl std::fmt::Display for AuthConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidValue(key, value) => write!(f, "invalid {key}: {value:?}"),
        }
    }
}

impl std::error::Error for AuthConfigError {}
