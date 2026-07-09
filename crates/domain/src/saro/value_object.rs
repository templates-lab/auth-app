//! SARO value objects — small, validated, immutable pieces of the model.
//!
//! Each type here wraps a primitive with a construction rule so an invalid
//! value cannot exist: a [`Progress`] is always 0–100, a [`ControlDesignScore`]
//! is always 0–10, a [`ProbabilityImpactId`] is always the two-digit lookup key
//! for a real (probability, impact) pair. The scoring rules that consume them
//! stay pure and testable.

use super::catalog::{Impact, Probability, Severity};

/// The lookup key into the severity table: the probability score concatenated
/// with the impact score (e.g. probability 5, impact 3 → `"53"`).
///
/// Built only from real catalog variants, so it is always two digits and always
/// resolves to a cell of the 5×5 matrix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProbabilityImpactId(String);

impl ProbabilityImpactId {
    /// The key as a string, for use as a map/table lookup key.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The (probability score, impact score) pair the key encodes.
    pub fn scores(&self) -> (u8, u8) {
        // Constructed only via `From`, so exactly two ASCII digits.
        let bytes = self.0.as_bytes();
        (bytes[0] - b'0', bytes[1] - b'0')
    }
}

impl From<(Probability, Impact)> for ProbabilityImpactId {
    fn from((probability, impact): (Probability, Impact)) -> Self {
        Self(format!("{}{}", probability.value(), impact.value()))
    }
}

impl std::fmt::Display for ProbabilityImpactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The heat-map cell code: severity score, probability abbreviation, impact
/// abbreviation, and the slot within the cell (e.g. `4AE1`).
///
/// The slot disambiguates several risks that land in the same matrix cell, so a
/// code identifies both the cell and the risk's position inside it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeatmapCode(String);

impl HeatmapCode {
    /// Compose a code from the cell's severity, the driving probability and
    /// impact, and the risk's slot within the cell.
    pub fn new(severity: Severity, probability: Probability, impact: Impact, slot: u32) -> Self {
        Self(format!(
            "{}{}{}{}",
            severity.value(),
            probability.abbrev(),
            impact.abbrev(),
            slot
        ))
    }

    /// The composed code string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HeatmapCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A control-design score: the weighted sum of the eight design parameters,
/// bounded to the 0–10 scale.
///
/// The eight per-parameter contributions are summed by an outer layer; this
/// value object guards the result's range so a score outside 0–10 — a sign the
/// weights were misconfigured — cannot be stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlDesignScore(f64);

impl ControlDesignScore {
    /// The inclusive upper bound of the scale.
    pub const MAX: f64 = 10.0;

    /// Wrap a pre-computed score, rejecting anything outside `0.0..=10.0` or a
    /// non-finite value.
    pub fn new(score: f64) -> Result<Self, ControlDesignScoreError> {
        if !score.is_finite() || !(0.0..=Self::MAX).contains(&score) {
            return Err(ControlDesignScoreError::OutOfRange(score));
        }
        Ok(Self(score))
    }

    /// Sum the eight weighted parameter contributions, then validate the range.
    ///
    /// A convenience over [`Self::new`] for callers holding the raw
    /// contributions rather than the total.
    pub fn from_parameters(parameters: [f64; 8]) -> Result<Self, ControlDesignScoreError> {
        Self::new(parameters.iter().sum())
    }

    /// The numeric score, 0.0–10.0.
    pub fn value(self) -> f64 {
        self.0
    }
}

/// Why a [`ControlDesignScore`] failed to construct.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlDesignScoreError {
    /// The score was non-finite or outside `0.0..=10.0`.
    OutOfRange(f64),
}

impl std::fmt::Display for ControlDesignScoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange(v) => write!(f, "control-design score {v} is outside 0.0..=10.0"),
        }
    }
}

impl std::error::Error for ControlDesignScoreError {}

/// The residual (probability, impact) pair after a control's or plan's soundness
/// has been applied to the inherent risk.
///
/// It is the output of the adjustment step: a strong control lowers probability
/// and/or impact, a weak one leaves them unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdjustmentResult {
    /// The probability after adjustment.
    pub probability: Probability,
    /// The impact after adjustment.
    pub impact: Impact,
}

impl AdjustmentResult {
    /// Pair an adjusted probability and impact.
    pub const fn new(probability: Probability, impact: Impact) -> Self {
        Self {
            probability,
            impact,
        }
    }

    /// The severity-table lookup key for the adjusted pair.
    pub fn lookup_id(&self) -> ProbabilityImpactId {
        (self.probability, self.impact).into()
    }
}

/// A completion percentage, 0–100.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Progress(u8);

impl Progress {
    /// Nothing done yet.
    pub const fn zero() -> Self {
        Self(0)
    }

    /// Wrap a percentage, rejecting anything above 100.
    pub fn new(percent: u8) -> Result<Self, ProgressError> {
        if percent > 100 {
            return Err(ProgressError::AboveHundred(percent));
        }
        Ok(Self(percent))
    }

    /// The percentage, 0–100.
    pub const fn percent(self) -> u8 {
        self.0
    }

    /// Whether the tracked work is fully complete.
    pub const fn is_complete(self) -> bool {
        self.0 == 100
    }
}

impl Default for Progress {
    fn default() -> Self {
        Self::zero()
    }
}

/// Why a [`Progress`] failed to construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressError {
    /// The percentage exceeded 100.
    AboveHundred(u8),
}

impl std::fmt::Display for ProgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AboveHundred(p) => write!(f, "progress {p} exceeds 100%"),
        }
    }
}

impl std::error::Error for ProgressError {}

/// Provenance and audit metadata common to every SARO entity.
///
/// Optional throughout: a freshly-built entity may not know its timestamps yet
/// (the persistence adapter stamps them), so the builders leave them `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    /// Who created the entity.
    pub created_by: Option<String>,
    /// When it was created.
    pub created_at: Option<std::time::SystemTime>,
    /// When it was last updated.
    pub updated_at: Option<std::time::SystemTime>,
}

impl Metadata {
    /// Empty metadata — all fields unset.
    pub fn empty() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probability_impact_id_concatenates_scores() {
        let id = ProbabilityImpactId::from((Probability::Alta, Impact::Significativo));
        assert_eq!(id.as_str(), "53");
        assert_eq!(id.scores(), (5, 3));
    }

    #[test]
    fn heatmap_code_composes_severity_abbrevs_and_slot() {
        let code = HeatmapCode::new(Severity::Extremo, Probability::Alta, Impact::Extremo, 1);
        assert_eq!(code.as_str(), "4AE1");
    }

    #[test]
    fn control_design_score_bounds_the_scale() {
        assert_eq!(ControlDesignScore::new(7.5).unwrap().value(), 7.5);
        assert_eq!(ControlDesignScore::new(0.0).unwrap().value(), 0.0);
        assert_eq!(ControlDesignScore::new(10.0).unwrap().value(), 10.0);
        assert!(ControlDesignScore::new(10.1).is_err());
        assert!(ControlDesignScore::new(-1.0).is_err());
        assert!(ControlDesignScore::new(f64::NAN).is_err());
    }

    #[test]
    fn control_design_score_sums_eight_parameters() {
        let score = ControlDesignScore::from_parameters([1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0])
            .unwrap();
        assert_eq!(score.value(), 8.0);
        assert!(ControlDesignScore::from_parameters([2.0; 8]).is_err());
    }

    #[test]
    fn adjustment_result_exposes_lookup_key() {
        let adjusted = AdjustmentResult::new(Probability::Ocasional, Impact::Bajo);
        assert_eq!(adjusted.lookup_id().as_str(), "22");
    }

    #[test]
    fn progress_rejects_over_hundred() {
        assert_eq!(Progress::new(100).unwrap().percent(), 100);
        assert!(Progress::new(100).unwrap().is_complete());
        assert_eq!(Progress::default(), Progress::zero());
        assert!(Progress::new(101).is_err());
    }
}
