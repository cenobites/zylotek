use anyhow::{Result, anyhow};
use reqwest::Url;
use scraper::{ElementRef, Html, Selector};
use zylotek_domain::woods::value_objects::{SourceReference, WebUrl, WoodDetails, WoodImages};

use crate::ScrapedWood;

mod helpers;
mod selectors;

pub mod basic_information;
pub mod mechanical_properties;
pub mod physical_properties;
pub mod workabilities;

pub fn parse_wood(html: &str, source_url: &str) -> Result<ScrapedWood> {
    let document = Html::parse_document(html);
    let base_url = Url::parse(source_url)?;
    let species_id = species_id(source_url)?;
    let container = content_container(&document)?;

    let table_selector = selectors::query("table")?;
    if container.select(&table_selector).count() > 8 {
        log::warn!("Page tables were not being processed: {source_url}");
    }

    let basic_information = basic_information::parse(container)?;
    let workabilities = workabilities::parse(container)?;
    let physical_properties = physical_properties::parse(container)?;
    let mechanical_properties = mechanical_properties::parse(container)?;

    let image = |src: Option<String>| -> Result<Option<WebUrl>> {
        Ok(src
            .and_then(|src| absolute_image_url(&base_url, &src))
            .map(WebUrl::new)
            .transpose()?)
    };
    let sources = basic_information.images;
    let main_image = image(Some(sources.main))?.ok_or_else(|| anyhow!("image not found"))?;

    Ok(ScrapedWood {
        name: basic_information.name,
        scientific_name: basic_information.scientific_name,
        details: WoodDetails {
            family: Some(basic_information.family),
            collection_location: Some(basic_information.collection_location),
            common_names: basic_information.common_names,
            source: Some(SourceReference {
                species_id,
                page_url: WebUrl::new(source_url)?,
            }),
            description: basic_information.description,
            kiln_drying: basic_information.kiln_drying,
            images: WoodImages {
                main: Some(main_image),
                tree: image(sources.tree)?,
                log: image(sources.log)?,
                bark: image(sources.bark)?,
                tangential_cut: image(sources.tangential_cut)?,
                cross_section: image(sources.cross_section)?,
                radial_cut: image(sources.radial_cut)?,
                wood: image(sources.wood)?,
            },
            physical_properties,
            mechanical_properties,
            workabilities,
        },
    })
}

fn species_id(url: &str) -> Result<u64> {
    let url = Url::parse(url)?;

    url.query_pairs()
        .find(|(key, _)| key == "especieestudadaid")
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .ok_or_else(|| anyhow!("especieestudadaid not found in URL"))
}

fn absolute_image_url(base_url: &Url, src: &str) -> Option<String> {
    base_url.join(src).ok().map(|url| url.to_string())
}

fn content_container<'a>(document: &'a Html) -> Result<ElementRef<'a>> {
    // The species page is contained inside the Joomla component area.
    //
    // We select the component body structurally.
    //
    // If you inspect the exact HTML and have a more specific stable
    // container class/id, replace this selector with that selector.
    let candidates = [
        ".item-page",
        ".com_madeirasbrasileiras",
        "#component",
        "main",
    ];

    for css in candidates {
        let selector = Selector::parse(css).unwrap();

        if let Some(element) = document.select(&selector).next() {
            return Ok(element);
        }
    }

    let body_selector = selectors::query("body")?;
    document
        .select(&body_selector)
        .next()
        .ok_or_else(|| anyhow!("page content container not found"))
}

#[cfg(test)]
pub(crate) mod tests {
    use zylotek_domain::woods::value_objects::*;

    use super::*;

    pub(crate) const SPECIES_HTML: &str = include_str!("../../tests/fixtures/species.html");
    pub(crate) const SOURCE_URL: &str =
        "https://lpf.florestal.gov.br/pt-br/madeiras-brasileiras/especie?especieestudadaid=42";

    pub(crate) fn parse(html: &str) -> Result<ScrapedWood> {
        parse_wood(html, SOURCE_URL)
    }

    /// The fixture with `from` replaced by `to`; panics if `from` is absent.
    pub(crate) fn mutated(from: &str, to: &str) -> String {
        assert!(SPECIES_HTML.contains(from), "fixture lacks {from:?}");
        SPECIES_HTML.replacen(from, to, 1)
    }

    fn measurement<M: Measurement>(value: Option<M>) -> Option<f64> {
        value.map(|m| m.value())
    }

    #[test]
    fn parses_a_whole_species_page() {
        let wood = parse(SPECIES_HTML).unwrap();

        assert_eq!(wood.name.as_str(), "Muiracatiara-rajada");
        assert_eq!(wood.scientific_name.as_str(), "Astronium lecointei");

        let details = &wood.details;
        assert_eq!(details.family.as_ref().unwrap().as_str(), "Anacardiaceae");
        assert_eq!(details.collection_location.as_deref(), Some("Pará"));
        assert_eq!(
            details.common_names.as_slice(),
            ["Muiracatiara", "aroeira", "maracatiara"]
        );

        let source = details.source.as_ref().unwrap();
        assert_eq!(source.species_id, 42);
        assert_eq!(source.page_url.as_str(), SOURCE_URL);

        assert_eq!(details.description.tree.get("altura"), Some("30 m"));
        assert_eq!(
            details.description.general_characteristics.get("grã"),
            Some("irregular")
        );
        assert_eq!(
            details.description.end_uses.as_deref(),
            Some("Construção civil")
        );
        assert_eq!(
            details.description.preservation.as_deref(),
            Some("Alta durabilidade natural")
        );
        assert_eq!(
            details.description.workability_notes.as_deref(),
            Some("Fácil de trabalhar")
        );
        assert_eq!(
            details
                .description
                .heartwood_color_classification
                .as_deref(),
            Some("castanho")
        );
        assert_eq!(details.kiln_drying.schedule.get("umidade"), Some("80%"));
        assert_eq!(measurement(details.kiln_drying.duration), Some(12.5));
    }

    #[test]
    fn resolves_image_urls_against_the_page() {
        let images = parse(SPECIES_HTML).unwrap().details.images;
        let url = |image: Option<WebUrl>| image.unwrap().to_string();

        assert_eq!(
            url(images.main),
            "https://lpf.florestal.gov.br/images/madeiras/astronium-main.jpg"
        );
        assert_eq!(
            url(images.tree),
            "https://lpf.florestal.gov.br/images/madeiras/arvore.jpg"
        );
        assert_eq!(
            url(images.log),
            "https://lpf.florestal.gov.br/images/madeiras/tora.jpg"
        );
        assert_eq!(
            url(images.bark),
            "https://lpf.florestal.gov.br/images/madeiras/casca.jpg"
        );
        assert_eq!(
            url(images.tangential_cut),
            "https://lpf.florestal.gov.br/images/madeiras/tangencial.jpg"
        );
        assert_eq!(
            url(images.cross_section),
            "https://lpf.florestal.gov.br/images/madeiras/transversal.jpg"
        );
        assert_eq!(
            url(images.radial_cut),
            "https://lpf.florestal.gov.br/images/madeiras/radial.jpg"
        );
        assert_eq!(
            url(images.wood),
            "https://lpf.florestal.gov.br/images/madeiras/madeira.jpg"
        );
    }

    #[test]
    fn parses_physical_properties() {
        let physical = parse(SPECIES_HTML).unwrap().details.physical_properties;

        assert_eq!(measurement(physical.density.dry), Some(0.9));
        assert_eq!(measurement(physical.density.green), Some(1.1));
        assert_eq!(measurement(physical.density.basic), Some(0.8));
        assert_eq!(measurement(physical.density.apparent), Some(0.95));
        assert_eq!(measurement(physical.shrinkage.tangential), Some(7.5));
        assert_eq!(measurement(physical.shrinkage.radial), Some(5.0));
        assert_eq!(measurement(physical.shrinkage.volumetric), Some(12.5));
        assert_eq!(physical.shrinkage.tangential_radial_ratio(), Some(1.5));
    }

    #[test]
    fn parses_mechanical_properties_per_moisture_condition() {
        let mechanical = parse(SPECIES_HTML).unwrap().details.mechanical_properties;

        // Green specimens are the first row of each table, dry specimens the second.
        let cases = [
            (
                MoistureCondition::Green,
                [
                    1000.0, 120_000.0, 500.0, 90.0, 700.0, 800.0, 60.0, 70.0, 110.0, 150.0, 120.0,
                ],
            ),
            (
                MoistureCondition::Dry,
                [
                    1250.0, 150_000.0, 625.0, 112.0, 875.0, 1000.0, 75.0, 87.0, 137.0, 187.0, 150.0,
                ],
            ),
        ];

        for (condition, expected) in cases {
            let by = mechanical.by_condition(condition);
            let actual = [
                measurement(by.static_bending.modulus_of_rupture),
                measurement(by.static_bending.modulus_of_elasticity),
                measurement(by.compression.parallel_to_grain),
                measurement(by.compression.perpendicular_to_grain),
                measurement(by.janka_hardness.parallel_to_grain),
                measurement(by.janka_hardness.end_grain),
                measurement(by.tension_perpendicular_to_grain),
                measurement(by.cleavage_strength),
                measurement(by.shear_parallel_to_grain),
                measurement(by.nail_extraction.parallel_to_grain),
                measurement(by.nail_extraction.end_grain),
            ];

            assert_eq!(actual, expected.map(Some), "{condition:?}");
        }
    }

    #[test]
    fn parses_workabilities() {
        let workabilities = parse(SPECIES_HTML).unwrap().details.workabilities;
        let equipment = &workabilities.equipment_finishing;

        assert_eq!(equipment.molding_planer.number_of_samples, Some(10));
        assert_eq!(
            equipment.molding_planer.surface_finish,
            WorkabilityRating::Good
        );
        assert_eq!(equipment.molding_planer.defect, WorkabilityDefect::Fluffy);
        assert_eq!(
            equipment.molding_planer.equipment_settings,
            WorkabilityEquipmentSettings::Suitable
        );
        assert_eq!(
            equipment.narrow_belt_sander.surface_finish,
            WorkabilityRating::VeryGood
        );
        assert_eq!(
            equipment.narrow_belt_sander.defect,
            WorkabilityDefect::Flawless
        );
        assert_eq!(
            equipment.narrow_belt_sander.equipment_settings,
            WorkabilityEquipmentSettings::NotSuitable
        );
        assert_eq!(equipment.lathe.surface_finish, WorkabilityRating::Fair);
        assert_eq!(
            equipment.lathe.defect,
            WorkabilityDefect::CrushedAndRippedOff
        );
        assert_eq!(equipment.drill_bit.number_of_samples, None);
        assert_eq!(
            equipment.drill_bit.surface_finish,
            WorkabilityRating::Excellent
        );
        assert_eq!(equipment.drill_bit.defect, WorkabilityDefect::NotApplicable);

        let surface = &workabilities.surface_finishing;
        assert_eq!(surface.planer, WorkabilityRating::VeryGood);
        assert_eq!(surface.sandpaper, WorkabilityRating::Good);
        assert_eq!(surface.vise, WorkabilityRating::Fair);
        assert_eq!(surface.drill, WorkabilityRating::Poor);
        assert_eq!(
            surface.nail_crack,
            NailCrackBehavior::DoesNotCrackExceptCorners
        );

        assert_eq!(
            workabilities.treatabilities.heartwood,
            TreatabilityRating::ModeratelyHard
        );
        assert_eq!(
            workabilities.treatabilities.sapwood,
            TreatabilityRating::VeryEasy
        );
    }

    #[test]
    fn rejects_a_source_url_without_a_species_id() {
        for url in [
            "https://lpf.florestal.gov.br/especie",
            "https://lpf.florestal.gov.br/especie?especieestudadaid=abc",
        ] {
            let error = parse_wood(SPECIES_HTML, url).unwrap_err();

            assert!(error.to_string().contains("especieestudadaid"), "{error}");
        }
    }

    #[test]
    fn rejects_an_unparsable_source_url() {
        assert!(parse_wood(SPECIES_HTML, "not a url").is_err());
    }

    #[test]
    fn falls_back_to_the_body_when_no_known_container_exists() {
        let html = SPECIES_HTML.replace(r#"class="item-page""#, "");

        assert_eq!(
            parse(&html).unwrap().scientific_name.as_str(),
            "Astronium lecointei"
        );
    }

    #[test]
    fn finds_the_content_in_any_known_container() {
        for (from, to) in [
            (r#"class="item-page""#, r#"class="com_madeirasbrasileiras""#),
            (r#"class="item-page""#, r#"id="component""#),
        ] {
            assert!(parse(&mutated(from, to)).is_ok(), "{to}");
        }
        let main = SPECIES_HTML
            .replacen(r#"<div class="item-page">"#, "<main>", 1)
            .replacen("</div>\n</body>", "</main>\n</body>", 1);
        assert!(parse(&main).is_ok());
    }

    #[test]
    fn empty_input_is_an_error_not_a_panic() {
        assert!(parse("").is_err());
        assert!(parse("<html></html>").is_err());
    }

    #[test]
    fn a_page_without_a_main_image_is_rejected_not_given_the_tree_photo() {
        let error = parse(&mutated(
            r#"<img src="/images/madeiras/astronium-main.jpg" alt="">"#,
            "",
        ))
        .unwrap_err();

        assert!(error.to_string().contains("image url not found"), "{error}");
    }

    #[test]
    fn a_page_without_any_image_is_rejected() {
        let html = regex_free_strip_images(SPECIES_HTML);

        let error = parse(&html).unwrap_err();

        assert!(error.to_string().contains("image url not found"), "{error}");
    }

    #[test]
    fn an_image_without_a_src_is_rejected_not_a_panic() {
        let error = parse(&mutated(
            r#"<img src="/images/madeiras/astronium-main.jpg" alt="">"#,
            r#"<img alt="">"#,
        ))
        .unwrap_err();

        assert!(
            error.to_string().contains("the main image has no src"),
            "{error}"
        );
    }

    fn regex_free_strip_images(html: &str) -> String {
        html.lines()
            .filter(|line| !line.contains("<img"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_page_without_the_species_name_is_rejected() {
        let error = parse(&mutated("<h4>Muiracatiara-rajada</h4>", "")).unwrap_err();

        assert!(
            error.to_string().contains("species name not found"),
            "{error}"
        );
    }

    #[test]
    fn missing_identification_fields_are_rejected() {
        for (fragment, expected) in [
            (
                "<p><span>Nome científico:</span> Astronium lecointei</p>",
                "scientific name not found",
            ),
            (
                "<p><span>Família:</span> Anacardiaceae</p>",
                "family not found",
            ),
            (
                "<p><span>Local de coleta:</span> Pará</p>",
                "collection location not found",
            ),
        ] {
            let error = parse(&mutated(fragment, "")).unwrap_err();

            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn invalid_values_are_rejected() {
        for (from, to) in [
            ("Astronium lecointei</p>", "Astronium</p>"),
            ("<td>0,90</td>", "<td>-0,90</td>"),
            ("<td>7,5</td>", "<td>101</td>"),
            ("<td>Bom</td>", "<td>Péssimo</td>"),
            ("<td>Felpuda</td>", "<td>Inexistente</td>"),
            ("<td>Adequada</td>", "<td>Talvez</td>"),
            ("<td>Não trinca, exceto cantos</td>", "<td>Depende</td>"),
            ("<td>Moderadamente difícil</td>", "<td>Impossível</td>"),
        ] {
            assert!(
                parse(&mutated(from, to)).is_err(),
                "{from} -> {to} must be rejected"
            );
        }
    }

    #[test]
    fn missing_tables_are_rejected() {
        for fragment in [
            "Propriedades físicas",
            "Flexão estática (kgf/cm²)",
            "Extração de pregos (kgf)",
        ] {
            assert!(
                parse(&mutated(fragment, "Outra coisa")).is_err(),
                "{fragment}"
            );
        }
    }

    #[test]
    fn every_mechanical_table_is_required() {
        for header in [
            "Flexão estática (kgf/cm²)",
            "Compressão (kgf/cm²)",
            "Dureza Janka (kgf)",
            "Tração (kgf/cm²)",
            "Fendilhamento (kgf/cm)",
            "Cisalhamento (kgf/cm²)",
            "Extração de pregos (kgf)",
        ] {
            let error = parse(&mutated(header, "Outra coisa")).unwrap_err();

            assert!(
                format!("{error:#}").contains("not found"),
                "{header}: {error:#}"
            );
        }
    }

    #[test]
    fn a_mechanical_table_without_a_dry_row_is_rejected() {
        let html = mutated("<tr><td>Seca</td><td>75</td></tr>", "");

        let error = parse(&html).unwrap_err();

        assert!(error.to_string().contains("Dry row missing"), "{error}");
    }

    #[test]
    fn optional_attributes_may_be_absent() {
        let mut html = SPECIES_HTML.to_owned();
        for label in [
            "Árvore:",
            "Características gerais:",
            "Secagem em estufa:",
            "Tempo de secagem (dias):",
            "Nomes comuns:",
            "Uso final:",
            "Preservação:",
            "Trabalhabilidade:",
            "Classificação da cor do cerne:",
        ] {
            html = html
                .lines()
                .filter(|line| !line.contains(&format!("<span>{label}</span>")))
                .collect::<Vec<_>>()
                .join("\n");
        }

        let details = parse(&html).unwrap().details;

        assert!(details.description.tree.is_empty());
        assert!(details.description.general_characteristics.is_empty());
        assert!(details.kiln_drying.schedule.is_empty());
        assert_eq!(details.kiln_drying.duration, None);
        assert!(details.common_names.is_empty());
        assert_eq!(details.description.end_uses, None);
    }

    #[test]
    fn an_invalid_kiln_drying_duration_is_rejected() {
        assert!(parse(&mutated("12,5", "-3")).is_err());
    }

    #[test]
    fn images_without_a_caption_and_unknown_rows_are_ignored() {
        let html = mutated(
            "<tr><td>Defeito</td>",
            "<tr><td>Observação</td><td>x</td></tr><tr><td>Defeito</td>",
        )
        .replacen(
            "<table",
            r##"<div><a href="#"><img src="/x.jpg" alt=""></a></div><table"##,
            1,
        );

        let wood = parse(&html).unwrap();

        assert_eq!(
            wood.details
                .workabilities
                .equipment_finishing
                .molding_planer
                .defect,
            WorkabilityDefect::Fluffy
        );
    }

    #[test]
    fn unprocessed_attributes_are_logged_as_a_warning() {
        let logger = capture::install();
        let html = mutated(
            "<p><span>Uso final:</span>",
            "<p><span>Novidade:</span> algo</p><p><span>Uso final:</span>",
        );

        parse(&html).unwrap();

        assert!(
            logger
                .lines()
                .iter()
                .any(|line| line.contains("Attributes were not processed")
                    && line.contains("novidade")),
            "captured log lines"
        );
    }

    #[test]
    fn a_page_with_many_tables_warns_that_some_are_not_processed() {
        let logger = capture::install();
        let extra = "<table><tr><td>x</td></tr></table>".repeat(8);
        let html = mutated("</div>\n</body>", &format!("{extra}</div>\n</body>"));

        parse(&html).unwrap();

        assert!(
            logger
                .lines()
                .iter()
                .any(|line| line.contains("Page tables were not being processed"))
        );
    }

    pub(crate) mod capture {
        use std::sync::{Mutex, OnceLock};

        pub(crate) struct Capture(Mutex<Vec<String>>);

        impl Capture {
            pub(crate) fn lines(&self) -> Vec<String> {
                self.0.lock().unwrap().clone()
            }
        }

        impl log::Log for Capture {
            fn enabled(&self, _: &log::Metadata) -> bool {
                true
            }

            fn log(&self, record: &log::Record) {
                self.0.lock().unwrap().push(record.args().to_string());
            }

            fn flush(&self) {}
        }

        #[test]
        fn the_capture_logger_accepts_every_record_and_flushes() {
            let capture = install();

            assert!(log::Log::enabled(
                capture,
                &log::Metadata::builder().build()
            ));
            log::Log::flush(capture);
        }

        pub(crate) fn install() -> &'static Capture {
            static CAPTURE: OnceLock<&'static Capture> = OnceLock::new();
            CAPTURE.get_or_init(|| {
                let capture: &'static Capture = Box::leak(Box::new(Capture(Mutex::default())));
                log::set_logger(capture).unwrap();
                log::set_max_level(log::LevelFilter::Info);
                capture
            })
        }
    }
}
