//! Server configuration, sourced entirely from the environment.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Runtime configuration for the HTTP server.
#[derive(Debug, Clone)]
pub(crate) struct Config {
    host: IpAddr,
    port: u16,
}

impl Config {
    /// Read configuration from the environment.
    ///
    /// - `APP_HOST` — bind address (default `0.0.0.0`)
    /// - `APP_PORT` — bind port (default `8080`)
    pub(crate) fn from_env() -> Result<Self, ConfigError> {
        let host = match std::env::var("APP_HOST") {
            Ok(raw) => raw.parse().map_err(|_| ConfigError::InvalidHost(raw))?,
            Err(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        let port = match std::env::var("APP_PORT") {
            Ok(raw) => raw.parse().map_err(|_| ConfigError::InvalidPort(raw))?,
            Err(_) => 8080,
        };
        Ok(Self { host, port })
    }

    /// The address the HTTP server should bind to.
    pub(crate) fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
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
