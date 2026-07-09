//! SARO entities — the four aggregates of the operational-risk model.
//!
//! [`OperationalRisk`], [`Control`], [`TreatmentPlan`], and [`RiskIndicator`]
//! each carry an opaque id newtype and hold catalog variants and value objects
//! rather than raw primitives, so an entity in memory is always internally
//! typed. Construction with required-field validation is the job of the
//! builders in [`crate::saro::builder`]; the plain `RiskIndicator::new` is the
//! one entity simple enough to build directly.

use super::catalog::{
    AssociatedRiskLevel, ControlAssignment, ControlDesign, ControlExecution, ControlImportance,
    ControlNature, ControlType, EventType, Frequency, Impact, IndicatorKind, Probability,
    RiskFactor, Soundness, TreatmentPriority,
};
use super::value_object::{ControlDesignScore, Metadata, Progress};

/// An opaque identifier for an [`OperationalRisk`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RiskId(String);

/// An opaque identifier for a [`Control`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ControlId(String);

/// An opaque identifier for a [`TreatmentPlan`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TreatmentPlanId(String);

/// An opaque identifier for a [`RiskIndicator`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndicatorId(String);

// The four id newtypes share the same shape; the macro keeps them consistent
// (wrap a string, borrow it back, `Display`) without four hand-copied blocks.
macro_rules! string_id {
    ($ty:ident) => {
        impl $ty {
            /// Wrap an identifier string.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// The identifier as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(RiskId);
string_id!(ControlId);
string_id!(TreatmentPlanId);
string_id!(IndicatorId);

/// The four associated-risk exposures an operational-risk event can trigger.
///
/// Every operational risk carries all four levels; a dimension that does not
/// apply is [`AssociatedRiskLevel::NoAplica`] rather than absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssociatedRisks {
    /// Legal / regulatory exposure.
    pub legal: AssociatedRiskLevel,
    /// Reputational exposure.
    pub reputational: AssociatedRiskLevel,
    /// Operational (process-continuity) exposure.
    pub operational: AssociatedRiskLevel,
    /// Contagion exposure to related entities.
    pub contagion: AssociatedRiskLevel,
}

impl Default for AssociatedRisks {
    fn default() -> Self {
        Self {
            legal: AssociatedRiskLevel::NoAplica,
            reputational: AssociatedRiskLevel::NoAplica,
            operational: AssociatedRiskLevel::NoAplica,
            contagion: AssociatedRiskLevel::NoAplica,
        }
    }
}

/// An operational risk: the inherent-risk assessment for one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationalRisk {
    /// Opaque unique identifier.
    pub id: RiskId,
    /// The risk factor(s) this risk originates from.
    pub factors: Vec<RiskFactor>,
    /// The three-level Supersolidaria event classification.
    pub event_type: EventType,
    /// Human-readable description of the risk.
    pub description: String,
    /// The causes that can trigger the event.
    pub causes: Vec<String>,
    /// The occurrence band, when assessed in words.
    pub frequency: Option<Frequency>,
    /// The inherent probability.
    pub probability: Probability,
    /// The inherent impact.
    pub impact: Impact,
    /// The associated-risk exposures.
    pub associated_risks: AssociatedRisks,
    /// Free-form analyst observations.
    pub observations: Option<String>,
    /// Provenance / audit metadata.
    pub metadata: Metadata,
}

/// A control mitigating an [`OperationalRisk`].
#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    /// Opaque unique identifier.
    pub id: ControlId,
    /// The risk this control mitigates.
    pub risk_id: RiskId,
    /// Human-readable description of the control.
    pub description: String,
    /// Whether the control has an owner.
    pub assignment: ControlAssignment,
    /// Preventive or corrective.
    pub control_type: ControlType,
    /// What the control does, in prose.
    pub functionality: Option<String>,
    /// Automatic or manual.
    pub nature: ControlNature,
    /// How often the control runs, in words.
    pub frequency: Option<String>,
    /// Where the control is documented.
    pub documentation: Option<String>,
    /// The concrete activities performed.
    pub activities: Option<String>,
    /// How well the control is designed.
    pub design: ControlDesign,
    /// How reliably it is actually executed.
    pub execution: ControlExecution,
    /// The resulting soundness (solidez).
    pub soundness: Soundness,
    /// The control's importance in the risk's control mix.
    pub importance: ControlImportance,
    /// The weighted design score, when computed.
    pub design_score: Option<ControlDesignScore>,
    /// Whether the control reduces the risk's probability.
    pub diminishes_probability: bool,
    /// Whether the control reduces the risk's impact.
    pub diminishes_impact: bool,
    /// Provenance / audit metadata.
    pub metadata: Metadata,
}

/// A treatment plan for a risk whose residual level is still unacceptable.
#[derive(Debug, Clone, PartialEq)]
pub struct TreatmentPlan {
    /// Opaque unique identifier.
    pub id: TreatmentPlanId,
    /// The risk this plan treats.
    pub risk_id: RiskId,
    /// What the plan covers.
    pub scope: String,
    /// The person accountable for the plan.
    pub responsible: String,
    /// The team executing it.
    pub team: Vec<String>,
    /// The budget allocated, if any.
    pub budget: Option<f64>,
    /// The plan's priority.
    pub priority: TreatmentPriority,
    /// How often progress is reviewed, in words.
    pub periodicity: Option<String>,
    /// Planned start.
    pub start_date: Option<std::time::SystemTime>,
    /// Planned end.
    pub end_date: Option<std::time::SystemTime>,
    /// Completion so far.
    pub progress: Progress,
    /// The soundness the risk reaches once the plan is in place.
    pub soundness_with_plan: Soundness,
    /// Whether the plan reduces the risk's probability.
    pub diminishes_probability: bool,
    /// Whether the plan reduces the risk's impact.
    pub diminishes_impact: bool,
    /// Provenance / audit metadata.
    pub metadata: Metadata,
}

/// A key control (KCI) or key risk (KRI) indicator monitoring a risk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskIndicator {
    /// Opaque unique identifier.
    pub id: IndicatorId,
    /// The risk this indicator monitors, when tied to one.
    pub risk_id: Option<RiskId>,
    /// KCI or KRI.
    pub kind: IndicatorKind,
    /// What the indicator measures.
    pub description: String,
    /// How the indicator is calculated.
    pub calculation: String,
    /// The unit of measurement.
    pub unit: String,
    /// How often it is measured, in words.
    pub frequency: String,
    /// The target (meta) value.
    pub target: String,
    /// Where the data comes from.
    pub source: String,
    /// Who owns the indicator.
    pub responsible: String,
    /// Provenance / audit metadata.
    pub metadata: Metadata,
}

impl RiskIndicator {
    /// Assemble a risk indicator from its already-typed parts.
    ///
    /// Unlike the three aggregates with builders, an indicator has no
    /// interdependent required fields beyond its own columns, so it is built
    /// directly.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: IndicatorId,
        risk_id: Option<RiskId>,
        kind: IndicatorKind,
        description: impl Into<String>,
        calculation: impl Into<String>,
        unit: impl Into<String>,
        frequency: impl Into<String>,
        target: impl Into<String>,
        source: impl Into<String>,
        responsible: impl Into<String>,
    ) -> Self {
        Self {
            id,
            risk_id,
            kind,
            description: description.into(),
            calculation: calculation.into(),
            unit: unit.into(),
            frequency: frequency.into(),
            target: target.into(),
            source: source.into(),
            responsible: responsible.into(),
            metadata: Metadata::empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_wrap_and_display() {
        assert_eq!(RiskId::new("r-1").as_str(), "r-1");
        assert_eq!(ControlId::new("c-1").to_string(), "c-1");
        assert_eq!(TreatmentPlanId::new("t-1").as_str(), "t-1");
        assert_eq!(IndicatorId::new("i-1").to_string(), "i-1");
    }

    #[test]
    fn associated_risks_default_to_not_applicable() {
        let ar = AssociatedRisks::default();
        assert_eq!(ar.legal, AssociatedRiskLevel::NoAplica);
        assert_eq!(ar.contagion, AssociatedRiskLevel::NoAplica);
    }

    #[test]
    fn risk_indicator_builds_directly() {
        let indicator = RiskIndicator::new(
            IndicatorId::new("i-1"),
            Some(RiskId::new("r-1")),
            IndicatorKind::Kri,
            "Fallos de conciliación",
            "conteo mensual",
            "eventos",
            "mensual",
            "0",
            "core bancario",
            "jefe de riesgos",
        );
        assert_eq!(indicator.kind, IndicatorKind::Kri);
        assert_eq!(indicator.risk_id, Some(RiskId::new("r-1")));
        assert_eq!(indicator.metadata, Metadata::empty());
    }
}
