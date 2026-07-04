//! Domain layer — the core of the hexagonal architecture.
//!
//! This crate holds the business model and the *ports* (traits) the outer
//! layers implement or consume. It depends on nothing but the standard
//! library: no web framework, no database driver. That purity is what the
//! dependency direction protects — `api → application → domain` and
//! `infrastructure → domain` both point inward, so the domain never reaches
//! outward to a framework.

pub mod health;

pub use health::{Health, HealthCheck, Readiness};
