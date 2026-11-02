//! Any bytes yield a parse result or an error, never a panic, and whatever parses is valid.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(parsed) = zylotek_sync::parse_open_data(data) else {
        return;
    };

    for wood in &parsed.woods {
        assert!(!wood.name.as_str().is_empty());
        assert!(wood.scientific_name.as_str().split(' ').count() >= 2);
        assert!(!wood.scientific_name.as_str().contains('='));
        assert!(wood.details.source.is_some());
    }
    for row in &parsed.rejected {
        assert!(!row.reason.is_empty());
    }
});
