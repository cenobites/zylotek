//! Malformed species pages must produce an error, never a panic.
//!
//! These cases were found by `fuzz/fuzz_targets/parse_wood.rs`, when the parsers `unwrap()`ed
//! and indexed into page content. A panic aborts the spawned sync task: the run reports that
//! page as failed, but with an opaque join error instead of a reason.

use std::panic::catch_unwind;

use zylotek_sync::parse_wood;

const SPECIES_HTML: &str = include_str!("fixtures/species.html");
const URL: &str = "https://lpf.florestal.gov.br/especie?especieestudadaid=42";

/// `html` with the first `<table>…</table>` after `marker` removed.
fn without_table_after(html: &str, marker: &str) -> String {
    let start = html.find(marker).expect("marker present") + marker.len();
    let table_start = start + html[start..].find("<table").expect("table after marker");
    let table_end = table_start + html[table_start..].find("</table>").unwrap() + "</table>".len();

    format!("{}{}", &html[..table_start], &html[table_end..])
}

fn assert_errors_without_panicking(html: &str) {
    let result = catch_unwind(|| parse_wood(html, URL)).expect("parse_wood must not panic");

    assert!(result.is_err(), "a malformed page must be rejected");
}

#[test]
fn the_fixture_itself_is_accepted() {
    assert!(parse_wood(SPECIES_HTML, URL).is_ok());
}

#[test]
fn a_page_without_any_table_is_rejected() {
    let mut html = SPECIES_HTML.to_owned();
    while let Some(start) = html.find("<table") {
        let end = start + html[start..].find("</table>").unwrap() + "</table>".len();
        html.replace_range(start..end, "");
    }

    assert_errors_without_panicking(&html);
}

#[test]
fn a_heading_without_its_table_is_rejected() {
    let html = without_table_after(SPECIES_HTML, "<h3>Propriedades físicas</h3>");

    assert_errors_without_panicking(&html);
}

#[test]
fn a_short_type_of_action_row_is_rejected() {
    let html = SPECIES_HTML.replace(
        "<tr><td>Resultado</td><td>Muito bom</td><td>Bom</td><td>Regular</td><td>Ruim</td><td>Não trinca, exceto cantos</td></tr>",
        "<tr><td>Resultado</td></tr>",
    );
    assert_ne!(html, SPECIES_HTML);

    assert_errors_without_panicking(&html);
}

#[test]
fn a_short_treatability_row_is_rejected() {
    let html = SPECIES_HTML.replace(
        "<tr><td>Moderadamente difícil</td><td>Muito fácil</td></tr>",
        "<tr><td>Moderadamente difícil</td></tr>",
    );
    assert_ne!(html, SPECIES_HTML);

    assert_errors_without_panicking(&html);
}
