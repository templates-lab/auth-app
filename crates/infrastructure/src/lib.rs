//! Infrastructure layer — adapters that implement domain ports.
//!
//! Depends on `domain` (for the port contracts) and, as the app grows, on
//! external drivers (database, cache, ...). It never depends on `application`
//! or `api`: dependencies point inward, toward the domain.

use domain::{Health, HealthCheck, Readiness};

/// A trivial [`HealthCheck`] adapter that always reports the service ready.
///
/// A real adapter would probe dependencies (database connectivity, pending
/// migrations, ...); this skeleton keeps the wiring honest without pulling in a
/// driver yet.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysReady;

impl HealthCheck for AlwaysReady {
    fn check(&self) -> Health {
        Health {
            readiness: Readiness::Ready,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_ready_reports_ready() {
        assert_eq!(AlwaysReady.check().readiness, Readiness::Ready);
    }
}
