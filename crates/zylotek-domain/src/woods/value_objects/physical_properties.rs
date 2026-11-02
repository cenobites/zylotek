use crate::woods::value_objects::measurements::{GramsPerCubicCentimeter, Measurement, Percentage};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhysicalProperties {
    pub density: Density,
    pub shrinkage: Shrinkage,
}

/// Density at different moisture conditions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Density {
    pub dry: Option<GramsPerCubicCentimeter>,
    pub green: Option<GramsPerCubicCentimeter>,
    pub basic: Option<GramsPerCubicCentimeter>,
    pub apparent: Option<GramsPerCubicCentimeter>,
}

/// Shrinkage from green to oven-dry condition.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shrinkage {
    pub tangential: Option<Percentage>,
    pub radial: Option<Percentage>,
    pub volumetric: Option<Percentage>,
}

impl Shrinkage {
    /// Tangential/radial shrinkage ratio (anisotropy coefficient). The closer
    /// to 1, the more dimensionally stable the wood.
    pub fn tangential_radial_ratio(&self) -> Option<f64> {
        let tangential = self.tangential?.value();
        let radial = self.radial?.value();

        (radial > 0.0).then(|| tangential / radial)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shrinkage(tangential: Option<f64>, radial: Option<f64>) -> Shrinkage {
        Shrinkage {
            tangential: tangential.map(|v| Percentage::new(v).unwrap()),
            radial: radial.map(|v| Percentage::new(v).unwrap()),
            volumetric: None,
        }
    }

    #[test]
    fn tangential_radial_ratio_divides_tangential_by_radial() {
        assert_eq!(
            shrinkage(Some(7.5), Some(5.0)).tangential_radial_ratio(),
            Some(1.5)
        );
    }

    #[test]
    fn tangential_radial_ratio_needs_both_values_and_a_positive_radial() {
        assert_eq!(shrinkage(None, Some(5.0)).tangential_radial_ratio(), None);
        assert_eq!(shrinkage(Some(7.5), None).tangential_radial_ratio(), None);
        assert_eq!(
            shrinkage(Some(7.5), Some(0.0)).tangential_radial_ratio(),
            None
        );
    }

    #[test]
    fn physical_properties_default_to_unknown() {
        let properties = PhysicalProperties::default();

        assert_eq!(properties.density, Density::default());
        assert_eq!(properties.shrinkage.clone(), Shrinkage::default());
        assert!(format!("{properties:?}").contains("density"));
    }
}
