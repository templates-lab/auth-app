//! Server configuration, sourced entirely from the environment.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

use api::oauth::OAuthRedirects;
use api::rate_limit::RateLimitConfig;
use domain::{LockoutPolicy, PasswordPolicy, ProviderId, SessionPolicy};
use infrastructure::{Argon2Params, OidcConfig, StripeConfig};

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
    /// The app-level rate limit on login attempts, per IP and per account.
    pub(crate) login_rate_limit: RateLimitConfig,
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
    /// Login rate limit (app-level, independent of Traefik's edge limit;
    /// applied separately per client IP and per submitted account email):
    /// - `LOGIN_RATE_LIMIT_MAX_REQUESTS` (default `10`)
    /// - `LOGIN_RATE_LIMIT_WINDOW_SECS` (default `60`)
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

        let login_rate_limit = RateLimitConfig {
            max_requests: parse_var("LOGIN_RATE_LIMIT_MAX_REQUESTS", 10)?,
            window: Duration::from_secs(parse_var("LOGIN_RATE_LIMIT_WINDOW_SECS", 60)?),
        };

        Ok(Self {
            argon2,
            password_policy,
            lockout_policy,
            session_policy,
            login_rate_limit,
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

/// OAuth/OIDC configuration: the set of providers to enable and where the
/// callback sends the browser afterward. Entirely optional — with no providers
/// configured, the OAuth endpoints simply return `404`.
#[derive(Debug, Clone)]
pub(crate) struct OAuthSettings {
    /// One [`OidcConfig`] per enabled provider.
    pub(crate) providers: Vec<OidcConfig>,
    /// The externally reachable origin the provider redirects back to.
    pub(crate) redirect_base: String,
    /// Where the callback sends the browser after success/failure.
    pub(crate) redirects: OAuthRedirects,
}

impl OAuthSettings {
    /// Read OAuth configuration from the environment.
    ///
    /// `OAUTH_PROVIDERS` is a comma-separated list of provider ids to enable
    /// (e.g. `google`). For each id `X`, the per-provider settings are read
    /// from `OAUTH_<X>_*` (uppercased):
    ///
    /// - `OAUTH_<X>_CLIENT_ID` / `_CLIENT_SECRET` (required)
    /// - `OAUTH_<X>_AUTH_ENDPOINT` / `_TOKEN_ENDPOINT` / `_USERINFO_ENDPOINT` /
    ///   `_ISSUER` (required)
    /// - `OAUTH_<X>_SCOPES` (comma-separated, default `openid,email`)
    ///
    /// Plus:
    /// - `OAUTH_REDIRECT_BASE` — the external origin (default
    ///   `http://localhost:8080`)
    /// - `OAUTH_SUCCESS_REDIRECT` (default `/`) / `OAUTH_FAILURE_REDIRECT`
    ///   (default `/login`)
    ///
    /// Returns `None` when `OAUTH_PROVIDERS` is unset or empty. A configured
    /// provider missing a required setting is an error, never a silent skip.
    pub(crate) fn from_env() -> Result<Option<Self>, OAuthConfigError> {
        let raw = std::env::var("OAUTH_PROVIDERS").unwrap_or_default();
        let ids: Vec<&str> = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if ids.is_empty() {
            return Ok(None);
        }

        let mut providers = Vec::with_capacity(ids.len());
        for id in ids {
            let provider_id = ProviderId::parse(id)
                .map_err(|e| OAuthConfigError(format!("invalid provider id {id:?}: {e}")))?;
            let key = |suffix: &str| format!("OAUTH_{}_{suffix}", id.to_ascii_uppercase());
            let required = |suffix: &str| -> Result<String, OAuthConfigError> {
                std::env::var(key(suffix))
                    .ok()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or_else(|| OAuthConfigError(format!("{} is required", key(suffix))))
            };
            let scopes = std::env::var(key("SCOPES"))
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "openid,email".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            providers.push(OidcConfig {
                provider_id,
                client_id: required("CLIENT_ID")?,
                client_secret: required("CLIENT_SECRET")?,
                auth_endpoint: required("AUTH_ENDPOINT")?,
                token_endpoint: required("TOKEN_ENDPOINT")?,
                userinfo_endpoint: required("USERINFO_ENDPOINT")?,
                issuer: required("ISSUER")?,
                scopes,
            });
        }

        let redirect_base = std::env::var("OAUTH_REDIRECT_BASE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "http://localhost:8080".to_string());
        let redirects = OAuthRedirects {
            success: std::env::var("OAUTH_SUCCESS_REDIRECT")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "/".to_string()),
            failure: std::env::var("OAUTH_FAILURE_REDIRECT")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "/login".to_string()),
        };

        Ok(Some(Self {
            providers,
            redirect_base,
            redirects,
        }))
    }
}

/// A missing or malformed OAuth configuration value.
#[derive(Debug)]
pub(crate) struct OAuthConfigError(String);

impl std::fmt::Display for OAuthConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "oauth configuration error: {}", self.0)
    }
}

impl std::error::Error for OAuthConfigError {}

/// Which payment provider is active — selected by environment, so switching
/// between the local `fake` and real `stripe` never recompiles any domain or
/// application logic (bead authapp-d47ce3).
#[derive(Debug, Clone)]
pub(crate) enum PaymentProviderConfig {
    /// No provider configured; the payment surface is inactive.
    Disabled,
    /// The deterministic in-memory provider (local dev / integration tests).
    Fake,
    /// The Stripe adapter, in whichever mode its secret key selects.
    Stripe(StripeConfig),
}

impl PaymentProviderConfig {
    /// Read the active provider from `PAYMENT_PROVIDER`:
    ///
    /// - unset / empty / `none` → [`Self::Disabled`]
    /// - `fake` → [`Self::Fake`]
    /// - `stripe` → [`Self::Stripe`], requiring `STRIPE_SECRET_KEY`
    ///   (`STRIPE_API_BASE` optional, defaults to `https://api.stripe.com`)
    ///
    /// An unrecognized value is an error, never a silent fallback.
    pub(crate) fn from_env() -> Result<Self, PaymentConfigError> {
        let raw = std::env::var("PAYMENT_PROVIDER").unwrap_or_default();
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "disabled" => Ok(Self::Disabled),
            "fake" => Ok(Self::Fake),
            "stripe" => {
                let secret_key = std::env::var("STRIPE_SECRET_KEY")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or_else(|| {
                        PaymentConfigError(
                            "STRIPE_SECRET_KEY is required for PAYMENT_PROVIDER=stripe".into(),
                        )
                    })?;
                let mut config = StripeConfig::new(secret_key);
                if let Some(base) = std::env::var("STRIPE_API_BASE")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
                {
                    config.api_base = base;
                }
                Ok(Self::Stripe(config))
            }
            other => Err(PaymentConfigError(format!(
                "unknown PAYMENT_PROVIDER {other:?} (expected fake|stripe|none)"
            ))),
        }
    }

    /// A short label for logs.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Fake => "fake",
            Self::Stripe(_) => "stripe",
        }
    }
}

/// A missing or malformed payment configuration value.
#[derive(Debug)]
pub(crate) struct PaymentConfigError(String);

impl std::fmt::Display for PaymentConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "payment configuration error: {}", self.0)
    }
}

impl std::error::Error for PaymentConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payment_provider_labels() {
        assert_eq!(PaymentProviderConfig::Disabled.label(), "disabled");
        assert_eq!(PaymentProviderConfig::Fake.label(), "fake");
        assert_eq!(
            PaymentProviderConfig::Stripe(StripeConfig::new("sk_test")).label(),
            "stripe"
        );
    }
}
