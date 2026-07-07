//! Money: an amount in a currency's smallest unit, never a float.

/// An amount of money, held as an integer count of the currency's smallest
/// unit (e.g. cents for `USD`) so arithmetic never suffers floating-point
/// rounding — the same reason payment processors themselves use minor units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Money {
    minor_units: i64,
    currency: Currency,
}

impl Money {
    /// Construct an amount from a whole count of minor units (e.g. cents).
    ///
    /// Rejects a negative amount: a payment for a negative amount is not a
    /// meaningful request at this layer (refund amounts are still expressed as
    /// positive quantities to refund, not negative charges).
    pub fn from_minor_units(minor_units: i64, currency: Currency) -> Result<Self, MoneyError> {
        if minor_units < 0 {
            return Err(MoneyError::Negative);
        }
        Ok(Self {
            minor_units,
            currency,
        })
    }

    /// The amount as a whole count of the currency's smallest unit.
    pub fn minor_units(&self) -> i64 {
        self.minor_units
    }

    /// The currency this amount is denominated in.
    pub fn currency(&self) -> Currency {
        self.currency
    }
}

/// Why a [`Money`] value could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoneyError {
    /// The minor-units amount was negative.
    Negative,
}

impl std::fmt::Display for MoneyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Negative => f.write_str("amount must not be negative"),
        }
    }
}

impl std::error::Error for MoneyError {}

/// An ISO 4217 currency code (`USD`, `EUR`, ...), stored as three uppercase
/// ASCII letters. Structural validation only — this is a stable lookup/display
/// key, not a registry of which codes are actually in circulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Currency([u8; 3]);

impl Currency {
    /// Parse a currency code, normalizing to uppercase.
    pub fn parse(raw: &str) -> Result<Self, CurrencyError> {
        if raw.len() != 3 || !raw.is_ascii() {
            return Err(CurrencyError::InvalidLength);
        }
        let mut bytes = [0u8; 3];
        for (i, b) in raw.bytes().enumerate() {
            if !b.is_ascii_alphabetic() {
                return Err(CurrencyError::NotAlphabetic);
            }
            bytes[i] = b.to_ascii_uppercase();
        }
        Ok(Self(bytes))
    }

    /// The code as an uppercase `&str`.
    pub fn as_str(&self) -> &str {
        // The bytes are always three uppercase ASCII letters (enforced at
        // construction), so this is always valid UTF-8.
        std::str::from_utf8(&self.0).expect("Currency always holds ASCII letters")
    }
}

impl std::fmt::Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a currency code failed to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrencyError {
    /// The input was not exactly three ASCII characters.
    InvalidLength,
    /// The input had three ASCII characters but not all were letters.
    NotAlphabetic,
}

impl std::fmt::Display for CurrencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLength => f.write_str("currency code must be exactly 3 ASCII letters"),
            Self::NotAlphabetic => f.write_str("currency code must contain only letters"),
        }
    }
}

impl std::error::Error for CurrencyError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_normalizes_case() {
        assert_eq!(Currency::parse("usd").unwrap().as_str(), "USD");
        assert_eq!(Currency::parse("Usd").unwrap().as_str(), "USD");
    }

    #[test]
    fn currency_rejects_malformed_codes() {
        assert_eq!(
            Currency::parse("US").unwrap_err(),
            CurrencyError::InvalidLength
        );
        assert_eq!(
            Currency::parse("USDD").unwrap_err(),
            CurrencyError::InvalidLength
        );
        assert_eq!(
            Currency::parse("U5D").unwrap_err(),
            CurrencyError::NotAlphabetic
        );
    }

    #[test]
    fn money_rejects_negative_amounts() {
        let usd = Currency::parse("USD").unwrap();
        assert_eq!(
            Money::from_minor_units(-1, usd).unwrap_err(),
            MoneyError::Negative
        );
        assert!(Money::from_minor_units(0, usd).is_ok());
    }

    #[test]
    fn money_exposes_its_parts() {
        let usd = Currency::parse("USD").unwrap();
        let amount = Money::from_minor_units(1_050, usd).unwrap();
        assert_eq!(amount.minor_units(), 1_050);
        assert_eq!(amount.currency(), usd);
    }
}
