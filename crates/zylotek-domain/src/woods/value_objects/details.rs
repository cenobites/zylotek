use std::{collections::BTreeMap, fmt};

use crate::woods::{
    errors::WoodError,
    value_objects::{
        measurements::Days,
        mechanical_properties::MechanicalProperties,
        names::{BotanicalFamily, CommonNames},
        physical_properties::PhysicalProperties,
        workabilities::Workabilities,
    },
};

/// Everything known about a wood besides its identity (name and scientific name).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WoodDetails {
    pub family: Option<BotanicalFamily>,
    pub collection_location: Option<String>,
    pub common_names: CommonNames,
    pub source: Option<SourceReference>,
    pub description: WoodDescription,
    pub kiln_drying: KilnDrying,
    pub images: WoodImages,
    pub physical_properties: PhysicalProperties,
    pub mechanical_properties: MechanicalProperties,
    pub workabilities: Workabilities,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WoodDescription {
    pub tree: Attributes,
    pub general_characteristics: Attributes,
    pub end_uses: Option<String>,
    pub preservation: Option<String>,
    pub workability_notes: Option<String>,
    pub heartwood_color_classification: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct KilnDrying {
    pub schedule: Attributes,
    pub duration: Option<Days>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WoodImages {
    pub main: Option<WebUrl>,
    pub tree: Option<WebUrl>,
    pub log: Option<WebUrl>,
    pub bark: Option<WebUrl>,
    pub tangential_cut: Option<WebUrl>,
    pub cross_section: Option<WebUrl>,
    pub radial_cut: Option<WebUrl>,
    pub wood: Option<WebUrl>,
}

/// Where the data was collected from: the LPF species page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReference {
    pub species_id: u64,
    pub page_url: WebUrl,
}

/// Free-form labelled attributes, e.g. tree height or general characteristics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attributes(BTreeMap<String, String>);

impl Attributes {
    pub fn new<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self(
            pairs
                .into_iter()
                .map(|(key, value)| {
                    (
                        key.as_ref().trim().to_owned(),
                        value.as_ref().trim().to_owned(),
                    )
                })
                .filter(|(key, _)| !key.is_empty())
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// An absolute http(s) URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WebUrl(String);

impl WebUrl {
    pub fn new(value: impl AsRef<str>) -> Result<Self, WoodError> {
        let value = value.as_ref().trim();
        let rest = value
            .strip_prefix("https://")
            .or_else(|| value.strip_prefix("http://"));

        match rest {
            Some(rest) if !rest.is_empty() && !rest.contains(char::is_whitespace) => {
                Ok(Self(value.to_owned()))
            }
            _ => Err(WoodError::InvalidUrl(value.to_owned())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WebUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn attributes_trim_keys_and_values_and_drop_blank_keys() {
        let attributes = Attributes::new([(" altura ", " 30 m "), ("  ", "ignored"), ("cor", "")]);

        assert_eq!(attributes.get("altura"), Some("30 m"));
        assert_eq!(attributes.get("cor"), Some(""));
        assert_eq!(attributes.get("missing"), None);
        assert_eq!(
            attributes.iter().collect::<Vec<_>>(),
            [("altura", "30 m"), ("cor", "")]
        );
        assert!(!attributes.is_empty());
        assert!(Attributes::default().is_empty());
    }

    #[test]
    fn attributes_keep_the_last_value_of_a_repeated_key() {
        let attributes = Attributes::new([("cor", "castanho"), ("cor ", "amarelo")]);

        assert_eq!(attributes.get("cor"), Some("amarelo"));
    }

    #[test]
    fn web_url_accepts_absolute_http_and_https_urls() {
        for value in [
            "https://lpf.florestal.gov.br/a.jpg",
            "http://example.com",
            " https://x.y ",
        ] {
            let url = WebUrl::new(value).unwrap();
            assert_eq!(url.as_str(), value.trim());
            assert_eq!(url.to_string(), value.trim());
        }
    }

    #[test]
    fn web_url_rejects_relative_empty_and_non_http_urls() {
        for value in [
            "",
            "https://",
            "http://",
            "/images/a.jpg",
            "ftp://host/a",
            "https://a b",
        ] {
            let error = WebUrl::new(value).unwrap_err();
            assert!(matches!(&error, WoodError::InvalidUrl(v) if v == value.trim()));
            assert_eq!(error.to_string(), format!("invalid url '{}'", value.trim()));
        }
    }

    #[test]
    fn details_default_to_empty() {
        let details = WoodDetails::default();

        assert_eq!(details.family, None);
        assert!(details.common_names.is_empty());
        assert_eq!(details.source, None);
        assert_eq!(details.description, WoodDescription::default());
        assert_eq!(details.kiln_drying, KilnDrying::default());
        assert_eq!(details.images, WoodImages::default());
        assert!(format!("{details:?}").contains("WoodDetails"));
    }

    #[test]
    fn source_reference_compares_by_value() {
        let source = SourceReference {
            species_id: 1,
            page_url: WebUrl::new("https://lpf.florestal.gov.br/?especieestudadaid=1").unwrap(),
        };

        assert_eq!(source.clone(), source);
        assert!(format!("{source:?}").contains("species_id: 1"));
    }

    proptest! {
        #[test]
        fn web_url_accepts_only_http_schemes_with_a_non_blank_rest(value in "(https?://)?[ a-z./]{0,12}") {
            let trimmed = value.trim();
            let rest = trimmed.strip_prefix("https://").or_else(|| trimmed.strip_prefix("http://"));
            let expected = rest.is_some_and(|rest| !rest.is_empty() && !rest.contains(char::is_whitespace));

            prop_assert_eq!(WebUrl::new(&value).is_ok(), expected);
        }
    }
}
