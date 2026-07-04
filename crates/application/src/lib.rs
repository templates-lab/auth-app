//! Application layer — use cases that orchestrate the domain.
//!
//! Depends only on `domain`. It receives domain ports (as trait objects or
//! generics) and exposes application services the delivery layer (`api`) calls.
//! It has no knowledge of HTTP, databases, or any framework.

use std::sync::Arc;

use domain::{Health, HealthCheck};

/// Application service exposing the health use case.
///
/// Constructed from any adapter implementing the [`HealthCheck`] domain port,
/// so the composition root decides which concrete adapter is injected.
#[derive(Clone)]
pub struct HealthService {
    check: Arc<dyn HealthCheck>,
}

// `dyn HealthCheck` is not `Debug`, so derive can't apply; the workspace lint
// `missing_debug_implementations` still wants a `Debug` impl on this public
// type, so provide an opaque one.
impl std::fmt::Debug for HealthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthService").finish_non_exhaustive()
    }
}

impl HealthService {
    /// Build the service from an adapter implementing the domain port.
    pub fn new(check: Arc<dyn HealthCheck>) -> Self {
        Self { check }
    }

    /// Report the current health of the service.
    pub fn health(&self) -> Health {
        self.check.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Health, Readiness};

    struct StubCheck(Readiness);

    impl HealthCheck for StubCheck {
        fn check(&self) -> Health {
            Health { readiness: self.0 }
        }
    }

    #[test]
    fn health_service_reports_the_ports_readiness() {
        let service = HealthService::new(Arc::new(StubCheck(Readiness::Ready)));
        assert_eq!(service.health().readiness, Readiness::Ready);

        let service = HealthService::new(Arc::new(StubCheck(Readiness::NotReady)));
        assert_eq!(service.health().readiness, Readiness::NotReady);
    }
}
