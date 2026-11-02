use std::fmt;

use crate::woods::errors::WoodError;

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

macro_rules! text_value_object {
    ($(#[$meta:meta])* $name:ident, $field:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }

            fn non_empty(value: &str) -> Result<String, WoodError> {
                let value = normalize(value);
                if value.is_empty() {
                    return Err(WoodError::EmptyText { field: $field });
                }

                Ok(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

text_value_object!(
    /// Popular (trade) name of a wood, e.g. "Muiracatiara-rajada".
    WoodName,
    "name"
);

text_value_object!(
    /// Binomial name of the species, e.g. "Astronium lecointei".
    ScientificName,
    "scientific name"
);

text_value_object!(
    /// Botanical family, e.g. "Anacardiaceae".
    BotanicalFamily,
    "family"
);

impl WoodName {
    pub fn new(value: impl AsRef<str>) -> Result<Self, WoodError> {
        Self::non_empty(value.as_ref()).map(Self)
    }
}

impl ScientificName {
    pub fn new(value: impl AsRef<str>) -> Result<Self, WoodError> {
        let value = Self::non_empty(value.as_ref())?;

        // A species name has at least a genus and a specific epithet ("sp." included).
        if value.split(' ').count() < 2 {
            return Err(WoodError::InvalidScientificName(value));
        }

        Ok(Self(value))
    }

    pub fn genus(&self) -> &str {
        self.0.split(' ').next().unwrap_or_default()
    }
}

impl BotanicalFamily {
    pub fn new(value: impl AsRef<str>) -> Result<Self, WoodError> {
        Self::non_empty(value.as_ref()).map(Self)
    }
}

/// Regional common names of a wood, trimmed and without duplicates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommonNames(Vec<String>);

impl CommonNames {
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut values: Vec<String> = Vec::new();
        for name in names {
            let name = normalize(name.as_ref());
            if name.is_empty() || values.iter().any(|v| v.eq_ignore_ascii_case(&name)) {
                continue;
            }
            values.push(name);
        }

        Self(values)
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn text_value_objects_collapse_whitespace() {
        assert_eq!(
            WoodName::new("  Muiracatiara \t rajada ").unwrap().as_str(),
            "Muiracatiara rajada"
        );
        assert_eq!(
            BotanicalFamily::new(" Anacardiaceae\n")
                .unwrap()
                .to_string(),
            "Anacardiaceae"
        );
    }

    #[test]
    fn text_value_objects_reject_blank_values() {
        for (error, field) in [
            (WoodName::new(" \t").unwrap_err(), "name"),
            (ScientificName::new("").unwrap_err(), "scientific name"),
            (BotanicalFamily::new("\n").unwrap_err(), "family"),
        ] {
            assert!(matches!(error, WoodError::EmptyText { field: f } if f == field));
            assert_eq!(error.to_string(), format!("{field} must not be empty"));
        }
    }

    #[test]
    fn scientific_name_needs_a_genus_and_an_epithet() {
        let error = ScientificName::new("Astronium").unwrap_err();

        assert!(matches!(&error, WoodError::InvalidScientificName(value) if value == "Astronium"));
        assert_eq!(
            error.to_string(),
            "scientific name 'Astronium' must have a genus and a specific epithet"
        );
    }

    #[test]
    fn scientific_name_exposes_its_genus() {
        let name = ScientificName::new("Astronium   lecointei").unwrap();

        assert_eq!(name.as_str(), "Astronium lecointei");
        assert_eq!(name.to_string(), "Astronium lecointei");
        assert_eq!(name.genus(), "Astronium");
        assert_eq!(ScientificName::new("Inga sp.").unwrap().genus(), "Inga");
    }

    #[test]
    fn common_names_are_normalized_and_deduplicated_ignoring_case() {
        let names = CommonNames::new([" Aroeira ", "", "aroeira", "Gonçalo  alves", "AROEIRA"]);

        assert_eq!(names.as_slice(), ["Aroeira", "Gonçalo alves"]);
        assert!(!names.is_empty());
        assert!(CommonNames::new(Vec::<String>::new()).is_empty());
    }

    proptest! {
        #[test]
        fn wood_name_normalization_is_idempotent(raw in "\\PC*") {
            if let Ok(name) = WoodName::new(&raw) {
                prop_assert_eq!(WoodName::new(name.as_str()).unwrap(), name.clone());
                prop_assert!(!name.as_str().starts_with(' ') && !name.as_str().ends_with(' '));
                prop_assert!(!name.as_str().contains("  "));
            } else {
                prop_assert!(raw.trim().is_empty());
            }
        }

        #[test]
        fn scientific_name_round_trips_through_display(genus in "[A-Z][a-z]{1,12}", epithet in "[a-z]{1,12}") {
            let name = ScientificName::new(format!("{genus} {epithet}")).unwrap();

            prop_assert_eq!(ScientificName::new(name.to_string()).unwrap(), name.clone());
            prop_assert_eq!(name.genus(), genus.as_str());
        }

        #[test]
        fn common_names_never_contain_case_insensitive_duplicates(raw in proptest::collection::vec("[a-zA-Z ]{0,8}", 0..12)) {
            let names = CommonNames::new(&raw);
            let lowered: Vec<String> = names.as_slice().iter().map(|n| n.to_lowercase()).collect();
            let mut unique = lowered.clone();
            unique.sort();
            unique.dedup();

            prop_assert_eq!(unique.len(), lowered.len());
            prop_assert!(names.as_slice().iter().all(|n| !n.is_empty()));
        }
    }
}
