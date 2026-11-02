use anyhow::{Context, Result, anyhow};
use zylotek_domain::woods::value_objects::Measurement;

pub(crate) fn parse_number(value: &str) -> Result<i64> {
    let value = value.trim();

    if value.is_empty() || value == "-" {
        return Err(anyhow!("missing numeric value"));
    }

    value
        .parse::<i64>()
        .with_context(|| format!("invalid numeric value: {value:?}"))
}

pub(crate) fn optional_number(value: Option<&String>) -> Option<i64> {
    let value = value?.trim();

    if value.is_empty() || value == "-" {
        return None;
    }

    parse_number(value).ok()
}

pub(crate) fn parse_decimal(value: &str) -> Result<f64> {
    let value = value.trim();

    if value.is_empty() || value == "-" {
        return Err(anyhow!("missing numeric value"));
    }

    value
        .replace(',', ".")
        .parse::<f64>()
        .with_context(|| format!("invalid numeric value: {value:?}"))
}

pub(crate) fn optional_decimal(value: Option<&String>) -> Option<f64> {
    let value = value?.trim();

    if value.is_empty() || value == "-" {
        return None;
    }

    parse_decimal(value).ok()
}

/// Parses an optional decimal cell into a domain measurement, rejecting out-of-range values.
pub(crate) fn measurement<M: Measurement>(value: Option<&String>) -> Result<Option<M>> {
    Ok(M::from_optional(optional_decimal(value))?)
}

pub(crate) fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use zylotek_domain::woods::value_objects::{Kgf, Percentage};

    use super::*;

    #[test]
    fn parse_number_reads_integers_and_rejects_the_rest() {
        assert_eq!(parse_number(" 42 ").unwrap(), 42);
        assert_eq!(parse_number("-7").unwrap(), -7);
        assert_eq!(
            parse_number("").unwrap_err().to_string(),
            "missing numeric value"
        );
        assert_eq!(
            parse_number(" - ").unwrap_err().to_string(),
            "missing numeric value"
        );
        assert!(
            parse_number("4.2")
                .unwrap_err()
                .to_string()
                .contains("invalid numeric value")
        );
    }

    #[test]
    fn parse_decimal_accepts_dots_and_brazilian_commas() {
        assert_eq!(parse_decimal("0,95").unwrap(), 0.95);
        assert_eq!(parse_decimal(" 12.5 ").unwrap(), 12.5);
        assert!(parse_decimal("-").is_err());
        assert!(parse_decimal("").is_err());
        assert!(parse_decimal("1,2,3").is_err());
        assert!(
            parse_decimal("abc")
                .unwrap_err()
                .to_string()
                .contains("invalid numeric value")
        );
    }

    #[test]
    fn optional_readers_turn_blanks_and_junk_into_none() {
        let some = |s: &str| Some(s.to_owned());

        assert_eq!(optional_number(some("10").as_ref()), Some(10));
        assert_eq!(optional_number(some("-").as_ref()), None);
        assert_eq!(optional_number(some("  ").as_ref()), None);
        assert_eq!(optional_number(some("x").as_ref()), None);
        assert_eq!(optional_number(None), None);

        assert_eq!(optional_decimal(some("1,5").as_ref()), Some(1.5));
        assert_eq!(optional_decimal(some("-").as_ref()), None);
        assert_eq!(optional_decimal(some("").as_ref()), None);
        assert_eq!(optional_decimal(some("x").as_ref()), None);
        assert_eq!(optional_decimal(None), None);
    }

    #[test]
    fn measurement_validates_the_range() {
        let some = |s: &str| Some(s.to_owned());

        assert_eq!(
            measurement::<Kgf>(some("150").as_ref())
                .unwrap()
                .unwrap()
                .value(),
            150.0
        );
        assert!(measurement::<Kgf>(None).unwrap().is_none());
        assert!(measurement::<Kgf>(some("-").as_ref()).unwrap().is_none());
        assert!(measurement::<Percentage>(some("100,5").as_ref()).is_err());
        assert!(measurement::<Kgf>(some("-1").as_ref()).is_err());
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(normalize("  a \t b\n c  "), "a b c");
        assert_eq!(normalize(""), "");
    }

    proptest! {
        #[test]
        fn readers_never_panic(input in "\\PC*") {
            let _ = parse_number(&input);
            let _ = parse_decimal(&input);
            let _ = optional_number(Some(&input));
            let _ = optional_decimal(Some(&input));
            let _ = measurement::<Kgf>(Some(&input));
        }

        #[test]
        fn normalize_is_idempotent(input in "\\PC*") {
            let once = normalize(&input);
            prop_assert_eq!(normalize(&once), once.clone());
        }

        #[test]
        fn formatted_decimals_round_trip(whole in 0u32..100_000, fraction in 0u32..100) {
            let value = format!("{whole},{fraction:02}");

            prop_assert_eq!(parse_decimal(&value).unwrap(), format!("{whole}.{fraction:02}").parse::<f64>().unwrap());
        }
    }
}
