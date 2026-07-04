//! The `Module` contract every pluggable feature implements.

use std::error::Error;
use std::fmt;

use axum::Router;

use crate::migration::Migration;

/// A pluggable feature module.
///
/// Each feature implements `Module` to expose itself to the composition root
/// through three seams — its migrations, its runtime initialization, and its
/// HTTP router — without any feature knowing about another. The registry holds
/// modules as `dyn Module`, so adding or removing a feature touches only the
/// module's own crate and its single registration line in the compositor.
pub trait Module: Send + Sync {
    /// Stable, unique identifier for the module.
    ///
    /// It namespaces the module's migrations (`<name>::<migration>`) and labels
    /// the module in diagnostics. It must be unique across registered modules.
    fn name(&self) -> &'static str;

    /// The module's migrations, in the order they must be applied.
    ///
    /// Defaults to none, for modules that own no schema.
    fn migrations(&self) -> Vec<Migration> {
        Vec::new()
    }

    /// Runtime initialization, run once by the compositor after migrations and
    /// before serving.
    ///
    /// Modules that need post-migration setup (seeding, background tasks, cache
    /// warmup, invariant checks) override this; returning `Err` aborts startup.
    /// The default is a no-op.
    fn init(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }

    /// The module's HTTP routes.
    ///
    /// The compositor merges this onto the application router. Defaults to an
    /// empty router for modules that expose no endpoints. A module scopes its
    /// own paths (e.g. under `/its-name`) so routes never collide with a peer's.
    fn router(&self) -> Router {
        Router::new()
    }
}

/// A module's [`Module::init`] hook failed, naming the offending module.
#[derive(Debug)]
pub struct InitError {
    /// The [`Module::name`] of the module whose init failed.
    pub module: String,
    /// The error the module's init returned.
    pub source: Box<dyn Error + Send + Sync>,
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "module {:?} failed to initialize: {}",
            self.module, self.source
        )
    }
}

impl Error for InitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}
