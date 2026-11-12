//! Fixed-point monetary amounts.
//!
//! Money is never stored as a float. An [`Amount`] is a signed integer that
//! counts ten-thousandths of a unit, so four decimal places are stored exactly.
//! All arithmetic is checked. An overflow returns an error to the caller. It
//! does not wrap and it does not panic.

use std::{fmt, str::FromStr};

use serde::de::{self, Deserialize, Deserializer, Visitor};
use thiserror::Error;

const DECIMAL_PLACES: usize = 4;
/// Number of minor units (ten-thousandths) in one whole unit.
const SCALE: i64 = 10_000;

/// A monetary amount with exactly four decimal places of precision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount(i64);

impl Amount {
    /// Builds an amount from a count of ten-thousandths, e.g. `15_000` is `1.5`.
    #[cfg(test)]
    pub const fn from_minor_units(units: i64) -> Self {
        Amount(units)
    }

    /// The amount as a count of ten-thousandths.
    #[cfg(test)]
    pub const fn minor_units(self) -> i64 {
        self.0
    }

    pub fn is_positive(self) -> bool {
        self.0 > 0
    }

    pub fn checked_add(self, other: Amount) -> Option<Amount> {
        self.0.checked_add(other.0).map(Amount)
    }

    pub fn checked_sub(self, other: Amount) -> Option<Amount> {
        self.0.checked_sub(other.0).map(Amount)
    }

    pub fn saturating_add(self, other: Amount) -> Amount {
        Amount(self.0.saturating_add(other.0))
    }
}

/// Always prints four decimal places, e.g. `1.5000` or `-0.0001`.
impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let magnitude = self.0.unsigned_abs();
        let scale = SCALE.unsigned_abs();
        write!(
            f,
            "{sign}{}.{:0width$}",
            magnitude / scale,
            magnitude % scale,
            width = DECIMAL_PLACES
        )
    }
}

/// The reason a string could not be parsed as an [`Amount`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseAmountError {
    #[error("not a decimal number")]
    Malformed,
    #[error("more than four decimal places")]
    TooPrecise,
    #[error("amount is too large")]
    OutOfRange,
}

impl FromStr for Amount {
    type Err = ParseAmountError;

    /// Parses a decimal number such as `12`, `12.5`, `.5` or `-0.0001`.
    ///
    /// Trailing zeros are accepted at any length because they do not change the
    /// value. A value that needs more than four decimal places is rejected. It
    /// is not rounded, because rounding would change the amount.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (negative, unsigned) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));

        let all_digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
        if (whole.is_empty() && fraction.is_empty()) || !all_digits(whole) || !all_digits(fraction)
        {
            return Err(ParseAmountError::Malformed);
        }

        let fraction = fraction.trim_end_matches('0');
        if fraction.len() > DECIMAL_PLACES {
            return Err(ParseAmountError::TooPrecise);
        }

        let parse_digits = |part: &str| -> Result<i64, ParseAmountError> {
            if part.is_empty() {
                Ok(0)
            } else {
                part.parse().map_err(|_| ParseAmountError::OutOfRange)
            }
        };
        // `fraction` has at most four digits, so the cast and the power cannot overflow.
        let fraction_scale = 10_i64.pow((DECIMAL_PLACES - fraction.len()) as u32);
        let fraction_units = parse_digits(fraction)? * fraction_scale;

        let units = parse_digits(whole)?
            .checked_mul(SCALE)
            .and_then(|units| units.checked_add(fraction_units))
            .ok_or(ParseAmountError::OutOfRange)?;

        Ok(Amount(if negative { -units } else { units }))
    }
}

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AmountVisitor;

        impl Visitor<'_> for AmountVisitor {
            type Value = Amount;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a decimal amount with at most four decimal places")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Amount, E> {
                value.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_str(AmountVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Amount, ParseAmountError> {
        s.parse()
    }

    #[test]
    fn parses_whole_and_fractional_values() {
        assert_eq!(parse("1"), Ok(Amount(10_000)));
        assert_eq!(parse("1."), Ok(Amount(10_000)));
        assert_eq!(parse("1.0"), Ok(Amount(10_000)));
        assert_eq!(parse("1.5"), Ok(Amount(15_000)));
        assert_eq!(parse(".5"), Ok(Amount(5_000)));
        assert_eq!(parse("0.0001"), Ok(Amount(1)));
        assert_eq!(parse("123.4567"), Ok(Amount(1_234_567)));
        assert_eq!(parse("007.05"), Ok(Amount(70_500)));
    }

    #[test]
    fn parses_negative_values() {
        assert_eq!(parse("-2.25"), Ok(Amount(-22_500)));
        assert_eq!(parse("-0.0001"), Ok(Amount(-1)));
    }

    #[test]
    fn accepts_lossless_trailing_zeros() {
        assert_eq!(parse("1.50000000"), Ok(Amount(15_000)));
    }

    #[test]
    fn rejects_values_needing_more_than_four_places() {
        assert_eq!(parse("1.00001"), Err(ParseAmountError::TooPrecise));
    }

    #[test]
    fn rejects_malformed_values() {
        for input in ["", ".", "-", "abc", "1.2.3", "1e5", "+1", "1,5", " 1", "--1"] {
            assert_eq!(parse(input), Err(ParseAmountError::Malformed), "{input:?}");
        }
    }

    #[test]
    fn rejects_values_that_do_not_fit() {
        assert_eq!(parse("922337203685477.5808"), Err(ParseAmountError::OutOfRange));
        assert_eq!(parse("99999999999999999999999"), Err(ParseAmountError::OutOfRange));
        assert_eq!(
            parse("922337203685477.5807"),
            Ok(Amount(i64::MAX)),
            "the largest representable amount still parses"
        );
    }

    #[test]
    fn displays_four_decimal_places() {
        assert_eq!(Amount(0).to_string(), "0.0000");
        assert_eq!(Amount(15_000).to_string(), "1.5000");
        assert_eq!(Amount(1).to_string(), "0.0001");
        assert_eq!(Amount(-1).to_string(), "-0.0001");
        assert_eq!(Amount(-150_000).to_string(), "-15.0000");
        assert_eq!(Amount(i64::MIN).to_string(), "-922337203685477.5808");
    }

    #[test]
    fn arithmetic_reports_overflow() {
        let max = Amount(i64::MAX);
        assert_eq!(max.checked_add(Amount(1)), None);
        assert_eq!(Amount(i64::MIN).checked_sub(Amount(1)), None);
        assert_eq!(Amount(1).checked_add(Amount(2)), Some(Amount(3)));
        assert_eq!(Amount(1).checked_sub(Amount(2)), Some(Amount(-1)));
    }
}
