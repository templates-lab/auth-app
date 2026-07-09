//! SARO — the Supersolidaria operational-risk (SARO) domain model.
//!
//! This is the foundational layer every other SARO task builds on: the closed
//! catalogs of §2 ([`catalog`]), the validated value objects that carry scores
//! and keys ([`value_object`]), the four aggregates ([`entity`]), the ports the
//! outer layers implement ([`port`]), and the builders that assemble an
//! aggregate only once its required fields are present ([`builder`]).
//!
//! Like the rest of this crate it is pure: catalog variants map to their
//! numeric scores, value objects guard their own invariants, and the ports
//! declare *what* persistence and severity resolution must do without reaching
//! for a framework or a driver.

pub mod builder;
pub mod catalog;
pub mod entity;
pub mod port;
pub mod value_object;

pub use builder::{BuilderError, ControlBuilder, RiskBuilder, TreatmentPlanBuilder};
pub use catalog::{
    AssociatedRiskLevel, CatalogError, ControlAssignment, ControlDesign, ControlExecution,
    ControlImportance, ControlNature, ControlType, EventType, EventTypeError, Frequency, Impact,
    ImplementationStatus, IndicatorKind, Probability, RiskFactor, Severity, Soundness,
    TreatmentPriority,
};
pub use entity::{
    AssociatedRisks, Control, ControlId, IndicatorId, OperationalRisk, RiskId, RiskIndicator,
    TreatmentPlan, TreatmentPlanId,
};
pub use port::{SaroRepository, SaroRepositoryError, SeverityLookup};
pub use value_object::{
    AdjustmentResult, ControlDesignScore, ControlDesignScoreError, HeatmapCode, Metadata,
    Progress, ProgressError, ProbabilityImpactId,
};
