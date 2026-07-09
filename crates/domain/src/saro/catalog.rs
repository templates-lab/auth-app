//! SARO closed catalogs — the enumerations of §2 of the SARO logic spec.
//!
//! Every type here is a *catálogo cerrado*: a fixed set of variants the
//! Supersolidaria operational-risk methodology recognizes. The ones the
//! methodology scores numerically ([`Probability`], [`Impact`], [`Severity`],
//! [`ControlType`], [`ControlNature`], [`Soundness`]) expose that number
//! through `value` / `from_value`; the purely nominal ones expose their stable
//! persistence string through `as_str` / `parse`. Both directions round-trip,
//! and both are unit-tested — a stored number or label that no variant covers
//! is a data-integrity fault surfaced as a [`CatalogError`], never a silent
//! default.
//!
//! Keeping these pure (no framework, no driver) is what lets the risk-scoring
//! rules be tested without a database or an HTTP layer.

/// A catalog value read back from storage matched no known variant.
///
/// Every write goes through a variant's `value` / `as_str`, so seeing this on
/// read means the underlying data drifted from the catalog — it is reported,
/// not swallowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// A numeric code outside the catalog's range.
    UnknownValue {
        /// The catalog that rejected the code (e.g. `"Probability"`).
        catalog: &'static str,
        /// The offending numeric code.
        value: i64,
    },
    /// A label string matching no variant.
    UnknownLabel {
        /// The catalog that rejected the label.
        catalog: &'static str,
        /// The offending label.
        label: String,
    },
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownValue { catalog, value } => {
                write!(f, "unknown {catalog} value {value}")
            }
            Self::UnknownLabel { catalog, label } => {
                write!(f, "unknown {catalog} label {label:?}")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// Probability of a risk materializing — the rows of the 5×5 heat map.
///
/// Numbered 1–5 (higher is more likely); the number is the score fed into the
/// severity lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Probability {
    /// Almost certain to occur (5).
    Alta,
    /// Occurs often (4).
    Frecuente,
    /// Can reasonably occur (3).
    Probable,
    /// Occurs occasionally (2).
    Ocasional,
    /// Unlikely to occur (1).
    Inferior,
}

impl Probability {
    /// The methodology's numeric score, 1–5.
    pub const fn value(self) -> u8 {
        match self {
            Self::Alta => 5,
            Self::Frecuente => 4,
            Self::Probable => 3,
            Self::Ocasional => 2,
            Self::Inferior => 1,
        }
    }

    /// The single-letter abbreviation used to build a [`crate::saro::HeatmapCode`].
    pub const fn abbrev(self) -> &'static str {
        match self {
            Self::Alta => "A",
            Self::Frecuente => "F",
            Self::Probable => "P",
            Self::Ocasional => "O",
            Self::Inferior => "I",
        }
    }

    /// Recover the variant from its numeric score.
    pub fn from_value(value: u8) -> Result<Self, CatalogError> {
        Ok(match value {
            5 => Self::Alta,
            4 => Self::Frecuente,
            3 => Self::Probable,
            2 => Self::Ocasional,
            1 => Self::Inferior,
            other => {
                return Err(CatalogError::UnknownValue {
                    catalog: "Probability",
                    value: other as i64,
                })
            }
        })
    }
}

/// Impact of a risk if it materializes — the columns of the 5×5 heat map.
///
/// Numbered 1–5 (higher is worse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Impact {
    /// Catastrophic consequence (5).
    Extremo,
    /// Major consequence (4).
    Alto,
    /// Notable consequence (3).
    Significativo,
    /// Minor consequence (2).
    Bajo,
    /// Negligible consequence (1).
    Insignificante,
}

impl Impact {
    /// The methodology's numeric score, 1–5.
    pub const fn value(self) -> u8 {
        match self {
            Self::Extremo => 5,
            Self::Alto => 4,
            Self::Significativo => 3,
            Self::Bajo => 2,
            Self::Insignificante => 1,
        }
    }

    /// The single-letter abbreviation used to build a [`crate::saro::HeatmapCode`].
    pub const fn abbrev(self) -> &'static str {
        match self {
            Self::Extremo => "E",
            Self::Alto => "A",
            Self::Significativo => "S",
            Self::Bajo => "B",
            Self::Insignificante => "I",
        }
    }

    /// Recover the variant from its numeric score.
    pub fn from_value(value: u8) -> Result<Self, CatalogError> {
        Ok(match value {
            5 => Self::Extremo,
            4 => Self::Alto,
            3 => Self::Significativo,
            2 => Self::Bajo,
            1 => Self::Insignificante,
            other => {
                return Err(CatalogError::UnknownValue {
                    catalog: "Impact",
                    value: other as i64,
                })
            }
        })
    }
}

/// Severity — the cell of the 5×5 matrix a (probability, impact) pair lands in.
///
/// Numbered 1–4 (higher is worse). It is the *output* of the severity lookup,
/// never scored directly from user input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// Extreme severity (4).
    Extremo,
    /// High severity (3).
    Alto,
    /// Moderate severity (2).
    Moderado,
    /// Low severity (1).
    Bajo,
}

impl Severity {
    /// The methodology's numeric score, 1–4.
    pub const fn value(self) -> u8 {
        match self {
            Self::Extremo => 4,
            Self::Alto => 3,
            Self::Moderado => 2,
            Self::Bajo => 1,
        }
    }

    /// Recover the variant from its numeric score.
    pub fn from_value(value: u8) -> Result<Self, CatalogError> {
        Ok(match value {
            4 => Self::Extremo,
            3 => Self::Alto,
            2 => Self::Moderado,
            1 => Self::Bajo,
            other => {
                return Err(CatalogError::UnknownValue {
                    catalog: "Severity",
                    value: other as i64,
                })
            }
        })
    }
}

/// The origin category of an operational risk (Basel-style risk factors).
///
/// A nominal catalog — no numeric score. Combinations of factors are recorded
/// per risk as more than one [`RiskFactor`] on the entity, so the catalog
/// itself stays the closed set of primary categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RiskFactor {
    /// People: staff errors, fraud, competency gaps.
    RecursoHumano,
    /// Processes: flawed or missing procedures.
    Procesos,
    /// Technology: systems, software, data.
    Tecnologia,
    /// Infrastructure: physical assets and facilities.
    Infraestructura,
    /// External factors: events outside the organization's control.
    FactoresExternos,
}

impl RiskFactor {
    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RecursoHumano => "recurso_humano",
            Self::Procesos => "procesos",
            Self::Tecnologia => "tecnologia",
            Self::Infraestructura => "infraestructura",
            Self::FactoresExternos => "factores_externos",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "recurso_humano" => Self::RecursoHumano,
            "procesos" => Self::Procesos,
            "tecnologia" => Self::Tecnologia,
            "infraestructura" => Self::Infraestructura,
            "factores_externos" => Self::FactoresExternos,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "RiskFactor",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// Textual occurrence bands, each mapping to a [`Probability`].
///
/// The methodology lets an analyst describe how often an event happens in
/// words; that description resolves to the numeric probability the heat map
/// needs via [`Self::to_probability`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Frequency {
    /// Continuous / permanent occurrence → [`Probability::Alta`].
    Continua,
    /// Frequent occurrence → [`Probability::Frecuente`].
    Frecuente,
    /// Periodic occurrence → [`Probability::Probable`].
    Periodica,
    /// Sporadic occurrence → [`Probability::Ocasional`].
    Esporadica,
    /// Rare occurrence → [`Probability::Inferior`].
    Rara,
}

impl Frequency {
    /// The probability this occurrence band resolves to.
    pub const fn to_probability(self) -> Probability {
        match self {
            Self::Continua => Probability::Alta,
            Self::Frecuente => Probability::Frecuente,
            Self::Periodica => Probability::Probable,
            Self::Esporadica => Probability::Ocasional,
            Self::Rara => Probability::Inferior,
        }
    }

    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Continua => "continua",
            Self::Frecuente => "frecuente",
            Self::Periodica => "periodica",
            Self::Esporadica => "esporadica",
            Self::Rara => "rara",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "continua" => Self::Continua,
            "frecuente" => Self::Frecuente,
            "periodica" => Self::Periodica,
            "esporadica" => Self::Esporadica,
            "rara" => Self::Rara,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "Frequency",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// Whether a control has an owner assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlAssignment {
    /// The control has a responsible owner.
    Asignado,
    /// The control has no owner yet.
    NoAsignado,
}

impl ControlAssignment {
    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Asignado => "asignado",
            Self::NoAsignado => "no_asignado",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "asignado" => Self::Asignado,
            "no_asignado" => Self::NoAsignado,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "ControlAssignment",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// Whether a control acts before or after the risk event. Scored 1–2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlType {
    /// Acts before the event to stop it (2).
    Preventivo,
    /// Acts after the event to correct it (1).
    Correctivo,
}

impl ControlType {
    /// The methodology's numeric score.
    pub const fn value(self) -> u8 {
        match self {
            Self::Preventivo => 2,
            Self::Correctivo => 1,
        }
    }

    /// Recover the variant from its numeric score.
    pub fn from_value(value: u8) -> Result<Self, CatalogError> {
        Ok(match value {
            2 => Self::Preventivo,
            1 => Self::Correctivo,
            other => {
                return Err(CatalogError::UnknownValue {
                    catalog: "ControlType",
                    value: other as i64,
                })
            }
        })
    }
}

/// Whether a control runs by machine or by hand. Scored 1–2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlNature {
    /// Runs automatically without human action (2).
    Automatico,
    /// Requires a person to execute it (1).
    Manual,
}

impl ControlNature {
    /// The methodology's numeric score.
    pub const fn value(self) -> u8 {
        match self {
            Self::Automatico => 2,
            Self::Manual => 1,
        }
    }

    /// Recover the variant from its numeric score.
    pub fn from_value(value: u8) -> Result<Self, CatalogError> {
        Ok(match value {
            2 => Self::Automatico,
            1 => Self::Manual,
            other => {
                return Err(CatalogError::UnknownValue {
                    catalog: "ControlNature",
                    value: other as i64,
                })
            }
        })
    }
}

/// How reliably a control is actually executed in practice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlExecution {
    /// Consistently executed as designed.
    Fuerte,
    /// Executed with gaps.
    Moderado,
    /// Rarely or poorly executed.
    Debil,
}

impl ControlExecution {
    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fuerte => "fuerte",
            Self::Moderado => "moderado",
            Self::Debil => "debil",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "fuerte" => Self::Fuerte,
            "moderado" => Self::Moderado,
            "debil" => Self::Debil,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "ControlExecution",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// How well a control is designed to address its risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlDesign {
    /// Fully addresses the risk.
    MuyAdecuado,
    /// Adequately addresses the risk.
    Adecuado,
    /// Does not adequately address the risk.
    Inadecuado,
}

impl ControlDesign {
    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MuyAdecuado => "muy_adecuado",
            Self::Adecuado => "adecuado",
            Self::Inadecuado => "inadecuado",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "muy_adecuado" => Self::MuyAdecuado,
            "adecuado" => Self::Adecuado,
            "inadecuado" => Self::Inadecuado,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "ControlDesign",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// Soundness (*solidez*) of a control or treatment plan. Scored 0/2/3.
///
/// The `Debil` case scores `0`: a weak control contributes nothing to reducing
/// the residual risk, which is why the score is not simply `1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Soundness {
    /// Strong — meaningfully reduces residual risk (3).
    Fuerte,
    /// Moderate reduction (2).
    Moderado,
    /// Weak — no effective reduction (0).
    Debil,
}

impl Soundness {
    /// The methodology's numeric score (0, 2, or 3).
    pub const fn value(self) -> u8 {
        match self {
            Self::Fuerte => 3,
            Self::Moderado => 2,
            Self::Debil => 0,
        }
    }

    /// Recover the variant from its numeric score.
    pub fn from_value(value: u8) -> Result<Self, CatalogError> {
        Ok(match value {
            3 => Self::Fuerte,
            2 => Self::Moderado,
            0 => Self::Debil,
            other => {
                return Err(CatalogError::UnknownValue {
                    catalog: "Soundness",
                    value: other as i64,
                })
            }
        })
    }
}

/// Whether a control or treatment plan actually exists yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImplementationStatus {
    /// Not started.
    NoExiste,
    /// Under construction.
    EnDesarrollo,
    /// Fully in place.
    Implementado,
}

impl ImplementationStatus {
    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoExiste => "no_existe",
            Self::EnDesarrollo => "en_desarrollo",
            Self::Implementado => "implementado",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "no_existe" => Self::NoExiste,
            "en_desarrollo" => Self::EnDesarrollo,
            "implementado" => Self::Implementado,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "ImplementationStatus",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// The urgency with which a risk must be treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreatmentPriority {
    /// Treat first.
    Alta,
    /// Treat next.
    Media,
    /// Treat when capacity allows.
    Baja,
}

impl TreatmentPriority {
    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alta => "alta",
            Self::Media => "media",
            Self::Baja => "baja",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "alta" => Self::Alta,
            "media" => Self::Media,
            "baja" => Self::Baja,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "TreatmentPriority",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// The level of an associated risk (legal, reputational, operational,
/// contagion) triggered by an operational-risk event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssociatedRiskLevel {
    /// High associated exposure.
    Alto,
    /// Moderate associated exposure.
    Medio,
    /// Low associated exposure.
    Bajo,
    /// The associated risk does not apply.
    NoAplica,
}

impl AssociatedRiskLevel {
    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alto => "alto",
            Self::Medio => "medio",
            Self::Bajo => "bajo",
            Self::NoAplica => "no_aplica",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "alto" => Self::Alto,
            "medio" => Self::Medio,
            "bajo" => Self::Bajo,
            "no_aplica" => Self::NoAplica,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "AssociatedRiskLevel",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// The relative importance of a control in the risk's control mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlImportance {
    /// Primary control for the risk.
    Alta,
    /// Supporting control.
    Media,
    /// Minor / compensating control.
    Baja,
}

impl ControlImportance {
    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Alta => "alta",
            Self::Media => "media",
            Self::Baja => "baja",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "alta" => Self::Alta,
            "media" => Self::Media,
            "baja" => Self::Baja,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "ControlImportance",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// Which family a [`crate::saro::RiskIndicator`] belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndicatorKind {
    /// Key Control Indicator — measures a control's health.
    Kci,
    /// Key Risk Indicator — measures a risk's exposure.
    Kri,
}

impl IndicatorKind {
    /// The stable string form persisted to storage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kci => "kci",
            Self::Kri => "kri",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, CatalogError> {
        Ok(match raw {
            "kci" => Self::Kci,
            "kri" => Self::Kri,
            other => {
                return Err(CatalogError::UnknownLabel {
                    catalog: "IndicatorKind",
                    label: other.to_string(),
                })
            }
        })
    }
}

/// The three-level Supersolidaria event-type classification (N1 → N2 → N3).
///
/// The methodology publishes the concrete catalog of codes as data, not as a
/// compiled table, so this is a value object holding one code per level rather
/// than a closed enum: each level is a non-empty, trimmed code, and N2 refines
/// N1 while N3 refines N2. Construction validates only that the path is fully
/// specified — the codes themselves are validated against the published catalog
/// by an outer layer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventType {
    n1: String,
    n2: String,
    n3: String,
}

impl EventType {
    /// Build a fully-specified N1/N2/N3 classification path.
    ///
    /// Each level is trimmed; an empty level at any tier is rejected, since the
    /// classification is hierarchical and a lower tier without its parent is
    /// meaningless.
    pub fn new(
        n1: impl Into<String>,
        n2: impl Into<String>,
        n3: impl Into<String>,
    ) -> Result<Self, EventTypeError> {
        let n1 = n1.into().trim().to_string();
        let n2 = n2.into().trim().to_string();
        let n3 = n3.into().trim().to_string();
        if n1.is_empty() {
            return Err(EventTypeError::EmptyLevel(1));
        }
        if n2.is_empty() {
            return Err(EventTypeError::EmptyLevel(2));
        }
        if n3.is_empty() {
            return Err(EventTypeError::EmptyLevel(3));
        }
        Ok(Self { n1, n2, n3 })
    }

    /// The level-1 (broadest) code.
    pub fn n1(&self) -> &str {
        &self.n1
    }

    /// The level-2 code.
    pub fn n2(&self) -> &str {
        &self.n2
    }

    /// The level-3 (most specific) code.
    pub fn n3(&self) -> &str {
        &self.n3
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.n1, self.n2, self.n3)
    }
}

/// Why an [`EventType`] failed to construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTypeError {
    /// The given hierarchy level (1, 2, or 3) was empty.
    EmptyLevel(u8),
}

impl std::fmt::Display for EventTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyLevel(level) => write!(f, "event-type level N{level} must not be empty"),
        }
    }
}

impl std::error::Error for EventTypeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probability_number_round_trips() {
        for p in [
            Probability::Alta,
            Probability::Frecuente,
            Probability::Probable,
            Probability::Ocasional,
            Probability::Inferior,
        ] {
            assert_eq!(Probability::from_value(p.value()).unwrap(), p);
        }
        assert_eq!(Probability::Alta.value(), 5);
        assert_eq!(Probability::Inferior.value(), 1);
    }

    #[test]
    fn impact_number_round_trips() {
        for i in [
            Impact::Extremo,
            Impact::Alto,
            Impact::Significativo,
            Impact::Bajo,
            Impact::Insignificante,
        ] {
            assert_eq!(Impact::from_value(i.value()).unwrap(), i);
        }
    }

    #[test]
    fn severity_number_round_trips() {
        for s in [
            Severity::Extremo,
            Severity::Alto,
            Severity::Moderado,
            Severity::Bajo,
        ] {
            assert_eq!(Severity::from_value(s.value()).unwrap(), s);
        }
    }

    #[test]
    fn control_scores_round_trip() {
        assert_eq!(ControlType::Preventivo.value(), 2);
        assert_eq!(ControlType::from_value(1).unwrap(), ControlType::Correctivo);
        assert_eq!(ControlNature::Automatico.value(), 2);
        assert_eq!(ControlNature::from_value(1).unwrap(), ControlNature::Manual);
    }

    #[test]
    fn soundness_weak_scores_zero_and_round_trips() {
        assert_eq!(Soundness::Debil.value(), 0);
        assert_eq!(Soundness::from_value(0).unwrap(), Soundness::Debil);
        assert_eq!(Soundness::from_value(3).unwrap(), Soundness::Fuerte);
    }

    #[test]
    fn out_of_range_numbers_are_rejected() {
        assert_eq!(
            Probability::from_value(6).unwrap_err(),
            CatalogError::UnknownValue {
                catalog: "Probability",
                value: 6,
            }
        );
        assert!(Severity::from_value(5).is_err());
        assert!(Soundness::from_value(1).is_err());
    }

    #[test]
    fn nominal_catalogs_round_trip_through_strings() {
        assert_eq!(
            RiskFactor::parse(RiskFactor::Tecnologia.as_str()).unwrap(),
            RiskFactor::Tecnologia
        );
        assert_eq!(
            ControlDesign::parse(ControlDesign::MuyAdecuado.as_str()).unwrap(),
            ControlDesign::MuyAdecuado
        );
        assert_eq!(
            AssociatedRiskLevel::parse(AssociatedRiskLevel::NoAplica.as_str()).unwrap(),
            AssociatedRiskLevel::NoAplica
        );
        assert_eq!(IndicatorKind::parse("kri").unwrap(), IndicatorKind::Kri);
    }

    #[test]
    fn unknown_labels_are_rejected() {
        assert_eq!(
            RiskFactor::parse("banana").unwrap_err(),
            CatalogError::UnknownLabel {
                catalog: "RiskFactor",
                label: "banana".to_string(),
            }
        );
    }

    #[test]
    fn frequency_maps_to_probability() {
        assert_eq!(Frequency::Continua.to_probability(), Probability::Alta);
        assert_eq!(Frequency::Rara.to_probability(), Probability::Inferior);
        assert_eq!(
            Frequency::parse("periodica").unwrap().to_probability(),
            Probability::Probable
        );
    }

    #[test]
    fn event_type_requires_every_level() {
        let et = EventType::new("N1", "N2", "N3").unwrap();
        assert_eq!(et.n1(), "N1");
        assert_eq!(et.to_string(), "N1/N2/N3");
        assert_eq!(
            EventType::new("N1", "  ", "N3").unwrap_err(),
            EventTypeError::EmptyLevel(2)
        );
    }
}
