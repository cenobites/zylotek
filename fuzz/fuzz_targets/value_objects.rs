//! Constructors never panic, and what they accept re-validates to itself.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use zylotek_domain::woods::{entities::WoodId, value_objects::*};

#[derive(Debug, Arbitrary)]
struct Input {
    text: String,
    other: String,
    names: Vec<String>,
    pairs: Vec<(String, String)>,
    number: f64,
}

fuzz_target!(|input: Input| {
    if let Ok(name) = WoodName::new(&input.text) {
        assert_eq!(WoodName::new(name.as_str()).unwrap(), name);
    }
    if let Ok(name) = ScientificName::new(&input.text) {
        assert_eq!(ScientificName::new(name.as_str()).unwrap(), name);
        assert!(!name.genus().is_empty());
    }
    if let Ok(family) = BotanicalFamily::new(&input.other) {
        assert_eq!(BotanicalFamily::new(family.as_str()).unwrap(), family);
    }
    if let Ok(url) = WebUrl::new(&input.text) {
        assert_eq!(WebUrl::new(url.as_str()).unwrap(), url);
    }
    let _ = input.text.parse::<WoodId>();

    let common = CommonNames::new(&input.names);
    assert_eq!(CommonNames::new(common.as_slice()), common);

    let attributes = Attributes::new(input.pairs.iter().map(|(k, v)| (k, v)));
    assert_eq!(Attributes::new(attributes.iter()), attributes);

    let valid = input.number.is_finite() && input.number >= 0.0;
    assert_eq!(Kgf::new(input.number).is_ok(), valid);
    assert_eq!(
        Percentage::new(input.number).is_ok(),
        valid && input.number <= 100.0
    );
    if let Ok(days) = Days::new(input.number) {
        assert_eq!(days.value(), input.number);
    }
});
