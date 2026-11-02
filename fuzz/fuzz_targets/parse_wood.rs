//! Whatever a species page contains, parsing returns a result — it never panics.

#![no_main]

use libfuzzer_sys::fuzz_target;

const URL: &str =
    "https://lpf.florestal.gov.br/pt-br/madeiras-brasileiras/especie?especieestudadaid=42";

fuzz_target!(|html: &str| {
    if let Ok(wood) = zylotek_sync::parse_wood(html, URL) {
        // A parsed wood only holds validated values.
        assert!(!wood.name.as_str().is_empty());
        assert!(wood.scientific_name.as_str().split(' ').count() >= 2);
        assert!(wood.details.images.main.is_some());
    }
});
