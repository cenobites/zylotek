//! What the website takes from strangers and from the scraped catalogue: the compare query
//! string, and wood JSON turned into display data. Nothing panics, accepted ids are safe
//! to put in a URL, and links that reach a page are web addresses.

#![no_main]

use libfuzzer_sys::fuzz_target;
use zylotek_contracts::woods::WoodResponse;
use zylotek_webapp::{
    browse::{Entry, Query},
    clients::is_id,
    i18n::Lang,
    pages::{compare_ids, escape},
    view,
};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    // Text written into the page by hand can never open a tag or an attribute.
    let escaped = escape(text);
    assert!(!escaped.contains(['<', '>', '"', '\'']));
    let decoded = escaped
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    assert_eq!(decoded, text, "escaping loses nothing");

    if let Ok(ids) = compare_ids(Some(text)) {
        assert!((2..=4).contains(&ids.len()));
        for (n, id) in ids.iter().enumerate() {
            assert!(is_id(id), "{id:?}");
            assert!(!ids[..n].contains(id), "duplicate {id:?}");
        }
    }

    // Whatever is in the address bar reads as a query without panicking.
    let query = Query::parse(Some(text));

    if let Ok(wood) = serde_json::from_str::<WoodResponse>(text) {
        for lang in [Lang::En, Lang::PtBr] {
            let _ = view::facts(&wood, lang);
            let _ = view::item(&Entry::new(&wood), &query, lang);
            let detail = view::detail(&wood, std::slice::from_ref(&wood), lang);
            for link in [&detail.image, &detail.source_url].into_iter().flatten() {
                assert!(link.starts_with("http://") || link.starts_with("https://"));
            }
        }
    }
});
