//! Arbitrary request bodies either fail to parse or convert to valid domain values.

#![no_main]

use libfuzzer_sys::fuzz_target;
use zylotek_api::woods::schemas::{WoodDetailsSchema, WoodRequest};
use zylotek_domain::woods::value_objects::{ScientificName, WoodDetails, WoodName};

fuzz_target!(|data: &[u8]| {
    let Ok(request) = serde_json::from_slice::<WoodRequest>(data) else {
        return;
    };

    let name = WoodName::new(&request.name);
    let scientific_name = ScientificName::new(&request.scientific_name);
    let Ok(details) = WoodDetails::try_from(request.details) else {
        return;
    };

    // Valid details survive a trip back through the response schema unchanged.
    let schema = WoodDetailsSchema::from(&details);
    let json = serde_json::to_vec(&schema).expect("responses serialize");
    let again: WoodDetailsSchema = serde_json::from_slice(&json).expect("responses deserialize");
    assert_eq!(
        WoodDetails::try_from(again).expect("valid details stay valid"),
        details
    );

    if let (Ok(name), Ok(scientific_name)) = (name, scientific_name) {
        assert_eq!(WoodName::new(name.as_str()).unwrap(), name);
        assert_eq!(
            ScientificName::new(scientific_name.as_str()).unwrap(),
            scientific_name
        );
    }
});
