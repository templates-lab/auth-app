//! contracts — the shared contract for pluggable feature modules.
//!
//! The backend is composed from independent feature *modules*. Each one
//! implements the [`Module`] trait to expose three seams: its migrations, its
//! runtime initialization, and its HTTP router. The composition root (`server`)
//! collects them in a [`ModuleRegistry`], runs their migrations (isolated per
//! module and ordered), initializes them, and merges their routers — all
//! through `dyn Module`, so it never depends on a concrete feature and no
//! feature depends on another.
//!
//! This crate is deliberately thin: it is the ONE thing every module shares. It
//! knows `axum` (a module's contract is to expose a `Router`) but holds no
//! domain model and no business logic.
//!
//! # Adding and removing modules
//!
//! Adding a feature is one line in the compositor —
//! `registry.register(my_feature::Module::new(..))` — plus the module's own
//! crate. Removing a feature is the reverse: drop its crate, its
//! `workspace.dependencies` entry, and that one registration line. Because
//! modules only ever meet through this crate, removing one cannot break the
//! compilation of the others.

mod migration;
mod module;
mod registry;

pub use migration::{
    InMemoryExecutor, Migration, MigrationError, MigrationExecutor, MigrationReport,
};
pub use module::{InitError, Module};
pub use registry::ModuleRegistry;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};

    /// A configurable stand-in used to exercise the registry without pulling in
    /// a real feature crate.
    #[derive(Debug)]
    struct StubModule {
        name: &'static str,
        migrations: Vec<Migration>,
        init_result: Result<(), &'static str>,
    }

    impl StubModule {
        fn new(name: &'static str, migrations: Vec<Migration>) -> Self {
            Self {
                name,
                migrations,
                init_result: Ok(()),
            }
        }

        fn failing_init(name: &'static str) -> Self {
            Self {
                name,
                migrations: Vec::new(),
                init_result: Err("boom"),
            }
        }
    }

    impl Module for StubModule {
        fn name(&self) -> &'static str {
            self.name
        }

        fn migrations(&self) -> Vec<Migration> {
            self.migrations.clone()
        }

        fn init(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.init_result.map_err(Into::into)
        }

        fn router(&self) -> Router {
            Router::new().route(&format!("/{}/ping", self.name), get(|| async { "pong" }))
        }
    }

    fn two_migrations() -> Vec<Migration> {
        vec![
            Migration::new("0001_init", "SQL_ONE"),
            Migration::new("0002_more", "SQL_TWO"),
        ]
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn migrations_run_isolated_per_module_and_in_order() {
        let registry = ModuleRegistry::new()
            .register(StubModule::new("alpha", two_migrations()))
            .register(StubModule::new("beta", two_migrations()));

        let mut executor = InMemoryExecutor::default();
        let report = registry.run_migrations(&mut executor).expect("first run");

        // Registration order across modules, declared order within each module,
        // and every id namespaced by its module — so `alpha`'s `0001` and
        // `beta`'s `0001` both apply without colliding.
        assert_eq!(
            executor.applied(),
            ids(&[
                "alpha::0001_init",
                "alpha::0002_more",
                "beta::0001_init",
                "beta::0002_more",
            ])
            .as_slice()
        );
        assert_eq!(report.applied.len(), 4);
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn rerunning_migrations_is_idempotent() {
        let registry = ModuleRegistry::new().register(StubModule::new("alpha", two_migrations()));

        let mut executor = InMemoryExecutor::default();
        registry.run_migrations(&mut executor).expect("first run");
        let second = registry.run_migrations(&mut executor).expect("second run");

        // Nothing pending the second time: the executor's record makes the run a
        // no-op that only reports skips.
        assert!(second.applied.is_empty());
        assert_eq!(
            second.skipped,
            ids(&["alpha::0001_init", "alpha::0002_more"])
        );
        assert_eq!(executor.applied().len(), 2);
    }

    #[test]
    fn duplicate_module_names_are_rejected_before_applying_anything() {
        let registry = ModuleRegistry::new()
            .register(StubModule::new("dup", two_migrations()))
            .register(StubModule::new("dup", two_migrations()));

        let mut executor = InMemoryExecutor::default();
        let error = registry.run_migrations(&mut executor).unwrap_err();

        assert!(matches!(error, MigrationError::DuplicateModule(name) if name == "dup"));
        assert!(
            executor.applied().is_empty(),
            "the uniqueness check must run before any migration is applied"
        );
    }

    #[test]
    fn init_runs_each_module_and_names_the_one_that_fails() {
        ModuleRegistry::new()
            .register(StubModule::new("healthy", Vec::new()))
            .init()
            .expect("a module whose init returns Ok must not fail the registry");

        let error = ModuleRegistry::new()
            .register(StubModule::failing_init("broken"))
            .init()
            .unwrap_err();
        assert_eq!(error.module, "broken");
    }

    #[test]
    fn router_merges_every_module_onto_the_base() {
        let registry = ModuleRegistry::new()
            .register(StubModule::new("alpha", Vec::new()))
            .register(StubModule::new("beta", Vec::new()));

        // Building the router without a panic proves the module routes merge
        // cleanly onto the base (no path collisions).
        let _app: Router = registry.router(Router::new().route("/health", get(|| async { "ok" })));
        assert_eq!(registry.modules().len(), 2);
    }
}
