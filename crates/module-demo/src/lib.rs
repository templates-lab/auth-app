//! A demonstration feature module.
//!
//! [`Demo`] implements [`contracts::Module`] to prove the plugin architecture
//! end to end: it owns two ordered migrations, initializes itself from injected
//! configuration, and mounts a small router under `/demo`. It depends on nothing
//! but the `contracts` crate, so it can be added to or removed from the
//! compositor without touching any other module.

use std::sync::Arc;

use axum::{extract::State, routing::get, Router};
use contracts::{Migration, Module};

/// The demo feature.
///
/// Constructing it *is* its initialization: the compositor injects
/// configuration (here, a greeting string) that the router then serves. Real
/// features inject their dependencies — a repository, a clock, a mailer — the
/// same way.
#[derive(Debug, Clone)]
pub struct Demo {
    greeting: Arc<str>,
}

impl Demo {
    /// Build the module with its injected configuration.
    pub fn new(greeting: impl Into<Arc<str>>) -> Self {
        Self {
            greeting: greeting.into(),
        }
    }
}

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn migrations(&self) -> Vec<Migration> {
        // Two migrations whose ordinal prefixes fix their order. The second
        // assumes the first has run — the registry guarantees exactly that,
        // isolated from every other module's schema history.
        vec![
            Migration::new(
                "0001_create_widgets",
                "CREATE TABLE demo_widgets (id UUID PRIMARY KEY);",
            ),
            Migration::new(
                "0002_widgets_add_label",
                "ALTER TABLE demo_widgets ADD COLUMN label TEXT NOT NULL DEFAULT '';",
            ),
        ]
    }

    fn router(&self) -> Router {
        // Routes are scoped under `/demo` so they never collide with the base
        // router or another module's paths.
        Router::new()
            .route("/demo/ping", get(ping))
            .route("/demo/greeting", get(greeting))
            .with_state(self.clone())
    }
}

/// Liveness of the demo module.
async fn ping() -> &'static str {
    "pong"
}

/// Return the greeting injected when the module was initialized.
async fn greeting(State(demo): State<Demo>) -> String {
    demo.greeting.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_two_migrations_in_declared_order() {
        let demo = Demo::new("hi");
        let names: Vec<_> = demo.migrations().iter().map(|m| m.name).collect();
        assert_eq!(names, ["0001_create_widgets", "0002_widgets_add_label"]);
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(Demo::new("hi").name(), "demo");
    }

    #[test]
    fn init_is_a_noop_ok() {
        assert!(Demo::new("hi").init().is_ok());
    }
}
