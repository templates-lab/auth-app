//! A minimal domain slice used to prove the architecture end to end.
//!
//! [`Readiness`] and [`Health`] are domain value objects; [`HealthCheck`] is a
//! *port* the application layer depends on and the infrastructure layer
//! implements. Real features (users, sessions, credentials) are added the same
//! way: a model plus the ports it needs.

/// Whether the service considers itself able to serve traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// The service and its dependencies are healthy.
    Ready,
    /// The service cannot currently serve traffic.
    NotReady,
}

/// The health of the service, expressed as a domain concept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Health {
    /// The readiness of the service.
    pub readiness: Readiness,
}

/// Port: something that can report the service's health.
///
/// Implemented by an adapter in the `infrastructure` crate and consumed by the
/// `application` crate. The domain only declares the contract — it never knows
/// how readiness is actually determined.
///
/// The method is `async` because a real adapter probes an I/O-bound dependency
/// (a Postgres `SELECT 1`, say). [`async_trait`](async_trait::async_trait) keeps
/// the trait object-safe so the application layer can still hold it behind
/// `Arc<dyn HealthCheck>`.
#[async_trait::async_trait]
pub trait HealthCheck: Send + Sync {
    /// Evaluate and return the current health of the service.
    async fn check(&self) -> Health;
}
