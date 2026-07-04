//! Module migrations and the executor that applies them.
//!
//! A [`Migration`] is a value a module owns; the [`ModuleRegistry`] namespaces
//! it under the module name and hands it to a [`MigrationExecutor`], which is
//! the seam a real database adapter fills. Ordering and isolation are the
//! registry's job (see [`ModuleRegistry::run_migrations`]); this module supplies
//! the pieces it drives.
//!
//! [`ModuleRegistry`]: crate::ModuleRegistry
//! [`ModuleRegistry::run_migrations`]: crate::ModuleRegistry::run_migrations

use std::error::Error;
use std::fmt;

/// A single, ordered schema change owned by a module.
///
/// The `name` is unique only *within* its module; the compositor namespaces it
/// as `<module>::<name>`, so two independent modules may each ship a
/// `0001_init` without colliding. That namespacing is what "applied in
/// isolation" means — one module's schema history can never shadow or reorder
/// another's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    /// Identifier, unique within the owning module. Conventionally an ordinal
    /// prefix (`0001_create_users`) so declaration order is also lexical order.
    pub name: &'static str,
    /// The schema change to apply, handed verbatim to the executor.
    pub sql: &'static str,
}

impl Migration {
    /// Construct a migration from its name and SQL.
    pub const fn new(name: &'static str, sql: &'static str) -> Self {
        Self { name, sql }
    }
}

/// Applies migrations to a backing store and records which have run.
///
/// This is the seam between the module system and real storage: the composition
/// root injects a concrete executor. Tests and the current skeleton use
/// [`InMemoryExecutor`]; a database-backed executor (tracking a
/// `schema_migrations` table) plugs in here once the `infrastructure` layer
/// grows a driver, with no change to any module.
///
/// Implementations MUST be idempotent: a re-applied `id` is recorded once, so
/// re-running the full plan only executes what is still pending.
pub trait MigrationExecutor {
    /// Whether the migration `id` (`<module>::<name>`) has already been applied.
    fn is_applied(&self, id: &str) -> bool;

    /// Apply the migration `id` with the given `sql`, recording it as applied.
    ///
    /// The error is returned verbatim to the caller, which wraps it with the
    /// failing `id` as [`MigrationError::Apply`].
    fn apply(&mut self, id: &str, sql: &str) -> Result<(), Box<dyn Error + Send + Sync>>;
}

/// An in-memory [`MigrationExecutor`] that records applied ids in order.
///
/// It runs no SQL — it exists to prove the runner's ordering and isolation and
/// to boot the server before a database adapter exists.
#[derive(Debug, Default)]
pub struct InMemoryExecutor {
    applied: Vec<String>,
}

impl InMemoryExecutor {
    /// The fully-qualified ids applied so far, in application order.
    pub fn applied(&self) -> &[String] {
        &self.applied
    }
}

impl MigrationExecutor for InMemoryExecutor {
    fn is_applied(&self, id: &str) -> bool {
        self.applied.iter().any(|applied| applied == id)
    }

    fn apply(&mut self, id: &str, _sql: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.applied.push(id.to_owned());
        Ok(())
    }
}

/// The outcome of running a migration plan: what was applied vs. already present.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// Fully-qualified ids applied this run, in order.
    pub applied: Vec<String>,
    /// Fully-qualified ids skipped because already applied, in order.
    pub skipped: Vec<String>,
}

/// A failure while applying migrations.
#[derive(Debug)]
pub enum MigrationError {
    /// Two registered modules report the same [`Module::name`], which would make
    /// their migration histories collide. Names must be unique.
    ///
    /// [`Module::name`]: crate::Module::name
    DuplicateModule(String),
    /// The executor failed to apply a migration.
    Apply {
        /// The fully-qualified id that failed (`<module>::<name>`).
        id: String,
        /// The underlying cause reported by the executor.
        source: Box<dyn Error + Send + Sync>,
    },
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateModule(name) => {
                write!(f, "duplicate module name {name:?}: names must be unique")
            }
            Self::Apply { id, source } => write!(f, "migration {id:?} failed: {source}"),
        }
    }
}

impl Error for MigrationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DuplicateModule(_) => None,
            Self::Apply { source, .. } => Some(source.as_ref()),
        }
    }
}
