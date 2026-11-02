use std::fmt;

use crate::woods::errors::WoodError;

/// A non-negative, finite physical quantity expressed in a fixed unit.
pub trait Measurement: Sized + Copy {
    const UNIT: &'static str;

    fn new(value: f64) -> Result<Self, WoodError>;

    fn value(&self) -> f64;

    fn from_optional(value: Option<f64>) -> Result<Option<Self>, WoodError> {
        value.map(Self::new).transpose()
    }
}

macro_rules! measurement {
    ($(#[$meta:meta])* $name:ident, $unit:literal, max = $max:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
        pub struct $name(f64);

        impl Measurement for $name {
            const UNIT: &'static str = $unit;

            fn new(value: f64) -> Result<Self, WoodError> {
                // NaN and infinities fall outside the range too.
                if !(0.0..=$max).contains(&value) {
                    return Err(WoodError::InvalidMeasurement {
                        unit: Self::UNIT,
                        value,
                    });
                }

                Ok(Self(value))
            }

            fn value(&self) -> f64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{} {}", self.0, Self::UNIT)
            }
        }
    };
    ($(#[$meta:meta])* $name:ident, $unit:literal) => {
        measurement!($(#[$meta])* $name, $unit, max = f64::MAX);
    };
}

measurement!(
    /// Density, in g/cm³.
    GramsPerCubicCentimeter,
    "g/cm³"
);

measurement!(
    /// A percentage between 0 and 100.
    Percentage,
    "%",
    max = 100.0
);

measurement!(
    /// Stress (bending, compression, tension, shear), in kgf/cm².
    KgfPerSquareCentimeter,
    "kgf/cm²"
);

measurement!(
    /// Cleavage strength, in kgf/cm.
    KgfPerCentimeter,
    "kgf/cm"
);

measurement!(
    /// Force (Janka hardness, nail extraction), in kgf.
    Kgf,
    "kgf"
);

measurement!(
    /// A duration in days.
    Days,
    "days"
);

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn assert_rejects<M: Measurement + fmt::Debug>(value: f64) {
        let error = M::new(value).unwrap_err();
        assert!(
            matches!(error, WoodError::InvalidMeasurement { unit, value: v } if unit == M::UNIT && (v == value || v.is_nan()))
        );
    }

    fn assert_measurement<M: Measurement + fmt::Debug + fmt::Display>(unit: &str, max: f64) {
        assert_eq!(M::UNIT, unit);
        assert_eq!(M::new(0.0).unwrap().value(), 0.0);
        assert_eq!(M::new(max).unwrap().value(), max);
        assert_eq!(M::new(1.5).unwrap().to_string(), format!("1.5 {unit}"));
        assert!(format!("{:?}", M::new(1.5).unwrap()).contains("1.5"));

        for invalid in [-0.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_rejects::<M>(invalid);
        }

        assert!(M::from_optional(None).unwrap().is_none());
        assert_eq!(M::from_optional(Some(2.0)).unwrap().unwrap().value(), 2.0);
        assert!(M::from_optional(Some(-1.0)).is_err());
    }

    #[test]
    fn every_measurement_validates_its_range_and_formats_with_its_unit() {
        assert_measurement::<GramsPerCubicCentimeter>("g/cm³", f64::MAX);
        assert_measurement::<Percentage>("%", 100.0);
        assert_measurement::<KgfPerSquareCentimeter>("kgf/cm²", f64::MAX);
        assert_measurement::<KgfPerCentimeter>("kgf/cm", f64::MAX);
        assert_measurement::<Kgf>("kgf", f64::MAX);
        assert_measurement::<Days>("days", f64::MAX);
    }

    #[test]
    fn percentage_rejects_values_above_one_hundred() {
        assert_rejects::<Percentage>(100.01);
        assert_eq!(
            Percentage::new(101.0).unwrap_err().to_string(),
            "invalid measurement: 101 %"
        );
    }

    #[test]
    fn measurements_compare_by_value() {
        let small = Kgf::new(1.0).unwrap();
        let large = Kgf::new(2.0).unwrap();

        assert!(small < large);
        assert_eq!(small, small.clone());
    }

    proptest! {
        #[test]
        fn percentage_accepts_exactly_zero_to_one_hundred(value in proptest::num::f64::ANY) {
            let accepted = Percentage::new(value).is_ok();

            prop_assert_eq!(accepted, (0.0..=100.0).contains(&value));
            if accepted {
                prop_assert_eq!(Percentage::new(value).unwrap().value(), value);
            }
        }

        #[test]
        fn unbounded_measurements_accept_every_finite_non_negative_value(value in proptest::num::f64::ANY) {
            prop_assert_eq!(Kgf::new(value).is_ok(), value.is_finite() && value >= 0.0);
        }
    }
}
