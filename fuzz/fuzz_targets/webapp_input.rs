//! What the website takes from strangers and from the scraped catalogue: the compare query
//! string, and wood JSON turned into display data. Nothing panics, accepted ids are safe
//! to put in a URL, and links that reach a page are web addresses.

#![no_main]

use libfuzzer_sys::fuzz_target;
use zylotek_contracts::woods::WoodResponse;
use zylotek_webapp::{clients::is_id, pages::compare_ids, view};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    if let Ok(ids) = compare_ids(Some(text)) {
        assert!((2..=4).contains(&ids.len()));
        for (n, id) in ids.iter().enumerate() {
            assert!(is_id(id), "{id:?}");
            assert!(!ids[..n].contains(id), "duplicate {id:?}");
        }
    }

    if let Ok(wood) = serde_json::from_str::<WoodResponse>(text) {
        let _ = view::facts(&wood);
        let _ = view::row(&wood);
        let detail = view::detail(&wood);
        for link in [&detail.image, &detail.source_url].into_iter().flatten() {
            assert!(link.starts_with("http://") || link.starts_with("https://"));
        }
    }
});
