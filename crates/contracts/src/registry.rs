//! The module registry: collects modules and drives their composition.

use std::collections::HashSet;
use std::fmt;

use axum::Router;

use crate::migration::{MigrationError, MigrationExecutor, MigrationReport};
use crate::module::{InitError, Module};

/// Collects the application's feature modules and composes them.
///
/// The composition root builds a registry by registering each module — one line
/// per feature — then asks it to run migrations, initialize the modules, and
/// merge their routes onto the base router. The registry only ever sees
/// `dyn Module`, so it stays ignorant of every concrete feature.
#[derive(Default)]
pub struct ModuleRegistry {
    modules: Vec<Box<dyn Module>>,
}

impl ModuleRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a feature module.
    ///
    /// Returns `self` so registrations chain, giving the compositor a
    /// one-line-per-module mount point.
    #[must_use]
    pub fn register(mut self, module: impl Module + 'static) -> Self {
        self.modules.push(Box::new(module));
        self
    }

    /// The registered modules, in registration order.
    pub fn modules(&self) -> &[Box<dyn Module>] {
        &self.modules
    }

    /// Merge every module's router onto `base`, in registration order.
    pub fn router(&self, base: Router) -> Router {
        self.modules
            .iter()
            .fold(base, |router, module| router.merge(module.router()))
    }

    /// Apply every module's pending migrations, isolated per module and in order.
    ///
    /// Ordering is deterministic: modules in registration order, and within each
    /// module its migrations in declared order. Each migration is namespaced as
    /// `<module>::<name>` before it reaches the executor, so modules cannot
    /// collide — that is the isolation guarantee. Module names must be unique; a
    /// duplicate is rejected before anything is applied.
    pub fn run_migrations(
        &self,
        executor: &mut dyn MigrationExecutor,
    ) -> Result<MigrationReport, MigrationError> {
        self.ensure_unique_names()?;

        let mut report = MigrationReport::default();
        for module in &self.modules {
            for migration in module.migrations() {
                let id = format!("{}::{}", module.name(), migration.name);
                if executor.is_applied(&id) {
                    report.skipped.push(id);
                    continue;
                }
                executor
                    .apply(&id, migration.sql)
                    .map_err(|source| MigrationError::Apply {
                        id: id.clone(),
                        source,
                    })?;
                report.applied.push(id);
            }
        }
        Ok(report)
    }

    /// Run each module's [`Module::init`] hook, in registration order.
    ///
    /// Runs after [`Self::run_migrations`] and before serving. The first module
    /// to fail aborts the sequence, and the returned [`InitError`] names it.
    pub fn init(&self) -> Result<(), InitError> {
        for module in &self.modules {
            module.init().map_err(|source| InitError {
                module: module.name().to_owned(),
                source,
            })?;
        }
        Ok(())
    }

    /// Reject two modules sharing a name, which would collide their migration
    /// histories. Checked before any migration runs so a bad registry applies
    /// nothing.
    fn ensure_unique_names(&self) -> Result<(), MigrationError> {
        let mut seen = HashSet::new();
        for module in &self.modules {
            if !seen.insert(module.name()) {
                return Err(MigrationError::DuplicateModule(module.name().to_owned()));
            }
        }
        Ok(())
    }
}

impl fmt::Debug for ModuleRegistry {
    // `dyn Module` is not `Debug`; list the modules by name instead.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleRegistry")
            .field(
                "modules",
                &self.modules.iter().map(|m| m.name()).collect::<Vec<_>>(),
            )
            .finish()
    }
}
