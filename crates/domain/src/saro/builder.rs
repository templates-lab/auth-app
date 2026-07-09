//! SARO builders — the validated construction path for the aggregates.
//!
//! An entity's fields are public (so an adapter can hydrate one from a row), but
//! the *authoring* path goes through a builder that refuses to produce an
//! aggregate missing a required field. Optional fields default sensibly
//! (associated risks default to *no aplica*, progress to zero, the reduction
//! flags to `false`); required fields must be set or [`build`](RiskBuilder::build)
//! returns a [`BuilderError`] naming exactly what is missing.

use super::catalog::{
    ControlAssignment, ControlDesign, ControlExecution, ControlImportance, ControlNature,
    ControlType, EventType, Frequency, Impact, Probability, RiskFactor, Soundness,
    TreatmentPriority,
};
use super::entity::{
    AssociatedRisks, Control, ControlId, OperationalRisk, RiskId, TreatmentPlan, TreatmentPlanId,
};
use super::value_object::{ControlDesignScore, Metadata, Progress};

/// Why a builder could not produce its aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuilderError {
    /// A required field was never set.
    MissingField(&'static str),
    /// A required text field was set but empty (after trimming).
    EmptyField(&'static str),
}

impl std::fmt::Display for BuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(name) => write!(f, "required field {name} is missing"),
            Self::EmptyField(name) => write!(f, "required field {name} must not be empty"),
        }
    }
}

impl std::error::Error for BuilderError {}

// Take a required `Option`, or return `BuilderError::MissingField`.
macro_rules! require {
    ($field:expr, $name:literal) => {
        $field.ok_or(BuilderError::MissingField($name))?
    };
}

/// Builds a validated [`OperationalRisk`].
#[derive(Debug, Default)]
pub struct RiskBuilder {
    id: Option<RiskId>,
    factors: Vec<RiskFactor>,
    event_type: Option<EventType>,
    description: Option<String>,
    causes: Vec<String>,
    frequency: Option<Frequency>,
    probability: Option<Probability>,
    impact: Option<Impact>,
    associated_risks: Option<AssociatedRisks>,
    observations: Option<String>,
    metadata: Metadata,
}

impl RiskBuilder {
    /// A fresh builder with no fields set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the id (required).
    pub fn id(mut self, id: RiskId) -> Self {
        self.id = Some(id);
        self
    }

    /// Add one risk factor (at least one is required).
    pub fn factor(mut self, factor: RiskFactor) -> Self {
        self.factors.push(factor);
        self
    }

    /// Replace the full set of risk factors.
    pub fn factors(mut self, factors: Vec<RiskFactor>) -> Self {
        self.factors = factors;
        self
    }

    /// Set the event-type classification (required).
    pub fn event_type(mut self, event_type: EventType) -> Self {
        self.event_type = Some(event_type);
        self
    }

    /// Set the description (required, non-empty).
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add one cause.
    pub fn cause(mut self, cause: impl Into<String>) -> Self {
        self.causes.push(cause.into());
        self
    }

    /// Set the occurrence frequency.
    pub fn frequency(mut self, frequency: Frequency) -> Self {
        self.frequency = Some(frequency);
        self
    }

    /// Set the inherent probability (required).
    pub fn probability(mut self, probability: Probability) -> Self {
        self.probability = Some(probability);
        self
    }

    /// Set the inherent impact (required).
    pub fn impact(mut self, impact: Impact) -> Self {
        self.impact = Some(impact);
        self
    }

    /// Set the associated-risk exposures (defaults to all *no aplica*).
    pub fn associated_risks(mut self, associated_risks: AssociatedRisks) -> Self {
        self.associated_risks = Some(associated_risks);
        self
    }

    /// Set free-form observations.
    pub fn observations(mut self, observations: impl Into<String>) -> Self {
        self.observations = Some(observations.into());
        self
    }

    /// Set the provenance metadata.
    pub fn metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Validate required fields and produce the [`OperationalRisk`].
    pub fn build(self) -> Result<OperationalRisk, BuilderError> {
        let description = require!(self.description, "description");
        if description.trim().is_empty() {
            return Err(BuilderError::EmptyField("description"));
        }
        if self.factors.is_empty() {
            return Err(BuilderError::MissingField("factors"));
        }
        Ok(OperationalRisk {
            id: require!(self.id, "id"),
            factors: self.factors,
            event_type: require!(self.event_type, "event_type"),
            description,
            causes: self.causes,
            frequency: self.frequency,
            probability: require!(self.probability, "probability"),
            impact: require!(self.impact, "impact"),
            associated_risks: self.associated_risks.unwrap_or_default(),
            observations: self.observations,
            metadata: self.metadata,
        })
    }
}

/// Builds a validated [`Control`].
#[derive(Debug, Default)]
pub struct ControlBuilder {
    id: Option<ControlId>,
    risk_id: Option<RiskId>,
    description: Option<String>,
    assignment: Option<ControlAssignment>,
    control_type: Option<ControlType>,
    functionality: Option<String>,
    nature: Option<ControlNature>,
    frequency: Option<String>,
    documentation: Option<String>,
    activities: Option<String>,
    design: Option<ControlDesign>,
    execution: Option<ControlExecution>,
    soundness: Option<Soundness>,
    importance: Option<ControlImportance>,
    design_score: Option<ControlDesignScore>,
    diminishes_probability: bool,
    diminishes_impact: bool,
    metadata: Metadata,
}

impl ControlBuilder {
    /// A fresh builder with no fields set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the id (required).
    pub fn id(mut self, id: ControlId) -> Self {
        self.id = Some(id);
        self
    }

    /// Set the mitigated risk (required).
    pub fn risk_id(mut self, risk_id: RiskId) -> Self {
        self.risk_id = Some(risk_id);
        self
    }

    /// Set the description (required, non-empty).
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set whether the control has an owner (required).
    pub fn assignment(mut self, assignment: ControlAssignment) -> Self {
        self.assignment = Some(assignment);
        self
    }

    /// Set preventive/corrective (required).
    pub fn control_type(mut self, control_type: ControlType) -> Self {
        self.control_type = Some(control_type);
        self
    }

    /// Set the prose functionality.
    pub fn functionality(mut self, functionality: impl Into<String>) -> Self {
        self.functionality = Some(functionality.into());
        self
    }

    /// Set automatic/manual (required).
    pub fn nature(mut self, nature: ControlNature) -> Self {
        self.nature = Some(nature);
        self
    }

    /// Set how often the control runs.
    pub fn frequency(mut self, frequency: impl Into<String>) -> Self {
        self.frequency = Some(frequency.into());
        self
    }

    /// Set where the control is documented.
    pub fn documentation(mut self, documentation: impl Into<String>) -> Self {
        self.documentation = Some(documentation.into());
        self
    }

    /// Set the concrete activities performed.
    pub fn activities(mut self, activities: impl Into<String>) -> Self {
        self.activities = Some(activities.into());
        self
    }

    /// Set the design adequacy (required).
    pub fn design(mut self, design: ControlDesign) -> Self {
        self.design = Some(design);
        self
    }

    /// Set the execution reliability (required).
    pub fn execution(mut self, execution: ControlExecution) -> Self {
        self.execution = Some(execution);
        self
    }

    /// Set the resulting soundness (required).
    pub fn soundness(mut self, soundness: Soundness) -> Self {
        self.soundness = Some(soundness);
        self
    }

    /// Set the control's importance (required).
    pub fn importance(mut self, importance: ControlImportance) -> Self {
        self.importance = Some(importance);
        self
    }

    /// Set the weighted design score.
    pub fn design_score(mut self, design_score: ControlDesignScore) -> Self {
        self.design_score = Some(design_score);
        self
    }

    /// Mark whether the control reduces probability.
    pub fn diminishes_probability(mut self, diminishes: bool) -> Self {
        self.diminishes_probability = diminishes;
        self
    }

    /// Mark whether the control reduces impact.
    pub fn diminishes_impact(mut self, diminishes: bool) -> Self {
        self.diminishes_impact = diminishes;
        self
    }

    /// Set the provenance metadata.
    pub fn metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Validate required fields and produce the [`Control`].
    pub fn build(self) -> Result<Control, BuilderError> {
        let description = require!(self.description, "description");
        if description.trim().is_empty() {
            return Err(BuilderError::EmptyField("description"));
        }
        Ok(Control {
            id: require!(self.id, "id"),
            risk_id: require!(self.risk_id, "risk_id"),
            description,
            assignment: require!(self.assignment, "assignment"),
            control_type: require!(self.control_type, "control_type"),
            functionality: self.functionality,
            nature: require!(self.nature, "nature"),
            frequency: self.frequency,
            documentation: self.documentation,
            activities: self.activities,
            design: require!(self.design, "design"),
            execution: require!(self.execution, "execution"),
            soundness: require!(self.soundness, "soundness"),
            importance: require!(self.importance, "importance"),
            design_score: self.design_score,
            diminishes_probability: self.diminishes_probability,
            diminishes_impact: self.diminishes_impact,
            metadata: self.metadata,
        })
    }
}

/// Builds a validated [`TreatmentPlan`].
#[derive(Debug, Default)]
pub struct TreatmentPlanBuilder {
    id: Option<TreatmentPlanId>,
    risk_id: Option<RiskId>,
    scope: Option<String>,
    responsible: Option<String>,
    team: Vec<String>,
    budget: Option<f64>,
    priority: Option<TreatmentPriority>,
    periodicity: Option<String>,
    start_date: Option<std::time::SystemTime>,
    end_date: Option<std::time::SystemTime>,
    progress: Option<Progress>,
    soundness_with_plan: Option<Soundness>,
    diminishes_probability: bool,
    diminishes_impact: bool,
    metadata: Metadata,
}

impl TreatmentPlanBuilder {
    /// A fresh builder with no fields set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the id (required).
    pub fn id(mut self, id: TreatmentPlanId) -> Self {
        self.id = Some(id);
        self
    }

    /// Set the treated risk (required).
    pub fn risk_id(mut self, risk_id: RiskId) -> Self {
        self.risk_id = Some(risk_id);
        self
    }

    /// Set the scope (required, non-empty).
    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// Set the accountable person (required, non-empty).
    pub fn responsible(mut self, responsible: impl Into<String>) -> Self {
        self.responsible = Some(responsible.into());
        self
    }

    /// Add one team member.
    pub fn team_member(mut self, member: impl Into<String>) -> Self {
        self.team.push(member.into());
        self
    }

    /// Set the allocated budget.
    pub fn budget(mut self, budget: f64) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Set the priority (required).
    pub fn priority(mut self, priority: TreatmentPriority) -> Self {
        self.priority = Some(priority);
        self
    }

    /// Set the review periodicity.
    pub fn periodicity(mut self, periodicity: impl Into<String>) -> Self {
        self.periodicity = Some(periodicity.into());
        self
    }

    /// Set the planned start.
    pub fn start_date(mut self, start_date: std::time::SystemTime) -> Self {
        self.start_date = Some(start_date);
        self
    }

    /// Set the planned end.
    pub fn end_date(mut self, end_date: std::time::SystemTime) -> Self {
        self.end_date = Some(end_date);
        self
    }

    /// Set the completion so far (defaults to zero).
    pub fn progress(mut self, progress: Progress) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Set the soundness reached once the plan is in place (required).
    pub fn soundness_with_plan(mut self, soundness: Soundness) -> Self {
        self.soundness_with_plan = Some(soundness);
        self
    }

    /// Mark whether the plan reduces probability.
    pub fn diminishes_probability(mut self, diminishes: bool) -> Self {
        self.diminishes_probability = diminishes;
        self
    }

    /// Mark whether the plan reduces impact.
    pub fn diminishes_impact(mut self, diminishes: bool) -> Self {
        self.diminishes_impact = diminishes;
        self
    }

    /// Set the provenance metadata.
    pub fn metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Validate required fields and produce the [`TreatmentPlan`].
    pub fn build(self) -> Result<TreatmentPlan, BuilderError> {
        let scope = require!(self.scope, "scope");
        if scope.trim().is_empty() {
            return Err(BuilderError::EmptyField("scope"));
        }
        let responsible = require!(self.responsible, "responsible");
        if responsible.trim().is_empty() {
            return Err(BuilderError::EmptyField("responsible"));
        }
        Ok(TreatmentPlan {
            id: require!(self.id, "id"),
            risk_id: require!(self.risk_id, "risk_id"),
            scope,
            responsible,
            team: self.team,
            budget: self.budget,
            priority: require!(self.priority, "priority"),
            periodicity: self.periodicity,
            start_date: self.start_date,
            end_date: self.end_date,
            progress: self.progress.unwrap_or_default(),
            soundness_with_plan: require!(self.soundness_with_plan, "soundness_with_plan"),
            diminishes_probability: self.diminishes_probability,
            diminishes_impact: self.diminishes_impact,
            metadata: self.metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event_type() -> EventType {
        EventType::new("N1", "N2", "N3").unwrap()
    }

    #[test]
    fn risk_builder_happy_path() {
        let risk = RiskBuilder::new()
            .id(RiskId::new("r-1"))
            .factor(RiskFactor::Procesos)
            .event_type(sample_event_type())
            .description("Fallo en conciliación de pagos")
            .cause("Falta de validación automática")
            .frequency(Frequency::Periodica)
            .probability(Probability::Probable)
            .impact(Impact::Alto)
            .build()
            .unwrap();

        assert_eq!(risk.id, RiskId::new("r-1"));
        assert_eq!(risk.factors, vec![RiskFactor::Procesos]);
        assert_eq!(risk.probability, Probability::Probable);
        assert_eq!(risk.causes.len(), 1);
        // Unset optional collections/associated risks fall back to defaults.
        assert_eq!(risk.associated_risks, AssociatedRisks::default());
    }

    #[test]
    fn risk_builder_reports_missing_required_fields() {
        let err = RiskBuilder::new()
            .id(RiskId::new("r-1"))
            .factor(RiskFactor::Procesos)
            .event_type(sample_event_type())
            .description("desc")
            .probability(Probability::Probable)
            // impact omitted
            .build()
            .unwrap_err();
        assert_eq!(err, BuilderError::MissingField("impact"));
    }

    #[test]
    fn risk_builder_rejects_missing_factor_and_empty_description() {
        let err = RiskBuilder::new()
            .id(RiskId::new("r-1"))
            .event_type(sample_event_type())
            .description("desc")
            .probability(Probability::Probable)
            .impact(Impact::Alto)
            .build()
            .unwrap_err();
        assert_eq!(err, BuilderError::MissingField("factors"));

        let err = RiskBuilder::new()
            .id(RiskId::new("r-1"))
            .factor(RiskFactor::Procesos)
            .event_type(sample_event_type())
            .description("   ")
            .probability(Probability::Probable)
            .impact(Impact::Alto)
            .build()
            .unwrap_err();
        assert_eq!(err, BuilderError::EmptyField("description"));
    }

    #[test]
    fn control_builder_happy_path_and_flags_default_false() {
        let control = ControlBuilder::new()
            .id(ControlId::new("c-1"))
            .risk_id(RiskId::new("r-1"))
            .description("Validación automática de saldos")
            .assignment(ControlAssignment::Asignado)
            .control_type(ControlType::Preventivo)
            .nature(ControlNature::Automatico)
            .design(ControlDesign::MuyAdecuado)
            .execution(ControlExecution::Fuerte)
            .soundness(Soundness::Fuerte)
            .importance(ControlImportance::Alta)
            .diminishes_probability(true)
            .build()
            .unwrap();

        assert_eq!(control.control_type, ControlType::Preventivo);
        assert!(control.diminishes_probability);
        assert!(!control.diminishes_impact);
        assert!(control.design_score.is_none());
    }

    #[test]
    fn control_builder_reports_missing_required_field() {
        let err = ControlBuilder::new()
            .id(ControlId::new("c-1"))
            .risk_id(RiskId::new("r-1"))
            .description("desc")
            .assignment(ControlAssignment::Asignado)
            .control_type(ControlType::Preventivo)
            // nature omitted
            .design(ControlDesign::Adecuado)
            .execution(ControlExecution::Moderado)
            .soundness(Soundness::Moderado)
            .importance(ControlImportance::Media)
            .build()
            .unwrap_err();
        assert_eq!(err, BuilderError::MissingField("nature"));
    }

    #[test]
    fn treatment_plan_builder_happy_path_defaults_progress() {
        let plan = TreatmentPlanBuilder::new()
            .id(TreatmentPlanId::new("t-1"))
            .risk_id(RiskId::new("r-1"))
            .scope("Automatizar la conciliación")
            .responsible("Jefe de Operaciones")
            .team_member("Analista 1")
            .priority(TreatmentPriority::Alta)
            .soundness_with_plan(Soundness::Fuerte)
            .diminishes_impact(true)
            .build()
            .unwrap();

        assert_eq!(plan.progress, Progress::zero());
        assert_eq!(plan.priority, TreatmentPriority::Alta);
        assert!(plan.diminishes_impact);
        assert_eq!(plan.team, vec!["Analista 1".to_string()]);
    }

    #[test]
    fn treatment_plan_builder_rejects_empty_responsible() {
        let err = TreatmentPlanBuilder::new()
            .id(TreatmentPlanId::new("t-1"))
            .risk_id(RiskId::new("r-1"))
            .scope("scope")
            .responsible("  ")
            .priority(TreatmentPriority::Media)
            .soundness_with_plan(Soundness::Moderado)
            .build()
            .unwrap_err();
        assert_eq!(err, BuilderError::EmptyField("responsible"));
    }
}
