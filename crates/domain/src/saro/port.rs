//! SARO ports — the traits the outer layers implement.
//!
//! Following the crate's hexagonal discipline, the domain declares *what*
//! persistence and severity resolution must offer and leaves *how* to the
//! `infrastructure` layer. Per this bead the ports are defined without an
//! implementation; a concrete Postgres [`SaroRepository`] and a table-backed
//! [`SeverityLookup`] arrive in later beads.

use super::catalog::{Impact, Probability, Severity};
use super::entity::{
    Control, ControlId, IndicatorId, OperationalRisk, RiskId, RiskIndicator, TreatmentPlan,
    TreatmentPlanId,
};

/// A storage failure from the [`SaroRepository`] port.
#[derive(Debug)]
pub enum SaroRepositoryError {
    /// The referenced entity did not exist.
    NotFound,
    /// An insert collided with an existing id.
    DuplicateId,
    /// Any other backend failure, described for logs.
    Backend(String),
}

impl std::fmt::Display for SaroRepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("saro entity not found"),
            Self::DuplicateId => f.write_str("saro entity id already exists"),
            Self::Backend(msg) => write!(f, "saro repository backend error: {msg}"),
        }
    }
}

impl std::error::Error for SaroRepositoryError {}

/// Port: persistence for the four SARO aggregates.
///
/// CRUD for [`OperationalRisk`], [`Control`], [`TreatmentPlan`], and
/// [`RiskIndicator`]. The domain names the operations the use cases need; the
/// adapter owns the SQL. Methods are `async` so the adapter can talk to a real
/// database without blocking the runtime.
#[async_trait::async_trait]
pub trait SaroRepository: Send + Sync {
    // --- OperationalRisk ---

    /// Insert a new operational risk. Rejects a duplicate id.
    async fn insert_risk(&self, risk: &OperationalRisk) -> Result<(), SaroRepositoryError>;

    /// Fetch an operational risk by id, if one exists.
    async fn get_risk(&self, id: &RiskId) -> Result<Option<OperationalRisk>, SaroRepositoryError>;

    /// List every operational risk.
    async fn list_risks(&self) -> Result<Vec<OperationalRisk>, SaroRepositoryError>;

    /// Overwrite an existing operational risk. Fails if it does not exist.
    async fn update_risk(&self, risk: &OperationalRisk) -> Result<(), SaroRepositoryError>;

    /// Delete an operational risk by id. Fails if it does not exist.
    async fn delete_risk(&self, id: &RiskId) -> Result<(), SaroRepositoryError>;

    // --- Control ---

    /// Insert a new control. Rejects a duplicate id.
    async fn insert_control(&self, control: &Control) -> Result<(), SaroRepositoryError>;

    /// Fetch a control by id, if one exists.
    async fn get_control(&self, id: &ControlId) -> Result<Option<Control>, SaroRepositoryError>;

    /// List every control mitigating a given risk.
    async fn list_controls_for_risk(
        &self,
        risk_id: &RiskId,
    ) -> Result<Vec<Control>, SaroRepositoryError>;

    /// Overwrite an existing control. Fails if it does not exist.
    async fn update_control(&self, control: &Control) -> Result<(), SaroRepositoryError>;

    /// Delete a control by id. Fails if it does not exist.
    async fn delete_control(&self, id: &ControlId) -> Result<(), SaroRepositoryError>;

    // --- TreatmentPlan ---

    /// Insert a new treatment plan. Rejects a duplicate id.
    async fn insert_treatment_plan(
        &self,
        plan: &TreatmentPlan,
    ) -> Result<(), SaroRepositoryError>;

    /// Fetch a treatment plan by id, if one exists.
    async fn get_treatment_plan(
        &self,
        id: &TreatmentPlanId,
    ) -> Result<Option<TreatmentPlan>, SaroRepositoryError>;

    /// List every treatment plan for a given risk.
    async fn list_treatment_plans_for_risk(
        &self,
        risk_id: &RiskId,
    ) -> Result<Vec<TreatmentPlan>, SaroRepositoryError>;

    /// Overwrite an existing treatment plan. Fails if it does not exist.
    async fn update_treatment_plan(
        &self,
        plan: &TreatmentPlan,
    ) -> Result<(), SaroRepositoryError>;

    /// Delete a treatment plan by id. Fails if it does not exist.
    async fn delete_treatment_plan(
        &self,
        id: &TreatmentPlanId,
    ) -> Result<(), SaroRepositoryError>;

    // --- RiskIndicator ---

    /// Insert a new risk indicator. Rejects a duplicate id.
    async fn insert_indicator(
        &self,
        indicator: &RiskIndicator,
    ) -> Result<(), SaroRepositoryError>;

    /// Fetch a risk indicator by id, if one exists.
    async fn get_indicator(
        &self,
        id: &IndicatorId,
    ) -> Result<Option<RiskIndicator>, SaroRepositoryError>;

    /// List every risk indicator monitoring a given risk.
    async fn list_indicators_for_risk(
        &self,
        risk_id: &RiskId,
    ) -> Result<Vec<RiskIndicator>, SaroRepositoryError>;

    /// Overwrite an existing risk indicator. Fails if it does not exist.
    async fn update_indicator(
        &self,
        indicator: &RiskIndicator,
    ) -> Result<(), SaroRepositoryError>;

    /// Delete a risk indicator by id. Fails if it does not exist.
    async fn delete_indicator(&self, id: &IndicatorId) -> Result<(), SaroRepositoryError>;
}

/// Port: the 5×5 severity matrix.
///
/// Resolves a (probability, impact) pair to the [`Severity`] cell it lands in.
/// The lookup is pure and total — every pair of catalog variants has a cell —
/// so it needs neither `async` nor a fallible return. The concrete table is
/// data owned by the implementing adapter.
pub trait SeverityLookup: Send + Sync {
    /// The severity cell for the given probability and impact.
    fn severity(&self, probability: Probability, impact: Impact) -> Severity;
}
