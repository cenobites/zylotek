//! Combines the open-data CSV with the scraped LPF pages into one wood per species.
//!
//! The CSV is the primary source: its measurements are the dataset's own, converted to the
//! domain's units. The web pages fill in what the CSV cannot hold (common names, images,
//! workabilities, the free-text notes) and any value a row left blank. Where both have a
//! value, the CSV's wins.

use std::collections::{HashMap, HashSet};

use zylotek_domain::woods::value_objects::*;

use crate::ScrapedWood;

/// The result of [`merge`].
#[derive(Debug, Default)]
pub struct Merged {
    /// Every CSV wood in its original order, then the web-only ones.
    pub woods: Vec<ScrapedWood>,
    /// How many CSV species were completed from a web page.
    pub enriched: usize,
    /// How many species only the web has, kept as they were scraped.
    pub web_only: usize,
}

/// Merges `scraped` into `open_data` by scientific name.
///
/// A species that appears on several CSV rows (one per test lot) keeps its first row, as
/// the import does, and later rows pass through untouched so the import can report them.
pub fn merge(open_data: Vec<ScrapedWood>, scraped: Vec<ScrapedWood>) -> Merged {
    let mut web: HashMap<String, ScrapedWood> = HashMap::new();
    for wood in scraped {
        web.entry(key(&wood.scientific_name)).or_insert(wood);
    }

    let mut merged = Merged::default();
    let mut seen = HashSet::new();
    for wood in open_data {
        let key = key(&wood.scientific_name);
        if !seen.insert(key.clone()) {
            merged.woods.push(wood);
            continue;
        }

        match web.remove(&key) {
            Some(page) => {
                merged.enriched += 1;
                merged.woods.push(fill_from(wood, page));
            }
            None => merged.woods.push(wood),
        }
    }

    // Keep the site's own order for what is left, which `HashMap` would shuffle.
    let mut web_only: Vec<_> = web.into_values().collect();
    web_only.sort_by(|a, b| a.scientific_name.as_str().cmp(b.scientific_name.as_str()));
    merged.web_only = web_only.len();
    merged.woods.extend(web_only);

    merged
}

/// The scientific name as the unique index compares it.
fn key(name: &ScientificName) -> String {
    name.as_str()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn fill_from(primary: ScrapedWood, page: ScrapedWood) -> ScrapedWood {
    ScrapedWood {
        name: primary.name,
        scientific_name: primary.scientific_name,
        details: primary.details.fill_gaps_from(page.details),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::parse_wood;

    const SPECIES_HTML: &str = include_str!("../tests/fixtures/species.html");
    const PAGE_URL: &str = "https://lpf.florestal.gov.br/especie?especieestudadaid=42";

    fn page() -> ScrapedWood {
        parse_wood(SPECIES_HTML, PAGE_URL).unwrap()
    }

    fn csv_wood(name: &str, scientific_name: &str) -> ScrapedWood {
        ScrapedWood {
            name: WoodName::new(name).unwrap(),
            scientific_name: ScientificName::new(scientific_name).unwrap(),
            details: WoodDetails::default(),
        }
    }

    fn density(value: f64) -> Option<GramsPerCubicCentimeter> {
        Some(GramsPerCubicCentimeter::new(value).unwrap())
    }

    #[test]
    fn a_csv_species_is_completed_from_its_page() {
        let web = page();
        let csv = csv_wood("Muiracatiara-rajada", &web.scientific_name.to_string());

        let merged = merge(vec![csv], vec![web.clone()]);

        assert_eq!((merged.enriched, merged.web_only), (1, 0));
        let wood = &merged.woods[0];
        assert_eq!(wood.name.as_str(), "Muiracatiara-rajada");
        assert_eq!(wood.details.common_names, web.details.common_names);
        assert_eq!(wood.details.images, web.details.images);
        assert_eq!(wood.details.workabilities, web.details.workabilities);
        assert_eq!(wood.details.source, web.details.source);
        assert_eq!(wood.details.family, web.details.family);
        assert_eq!(
            wood.details.mechanical_properties,
            web.details.mechanical_properties
        );
        assert_eq!(wood.details.description, web.details.description);
        assert_eq!(wood.details.kiln_drying, web.details.kiln_drying);
    }

    #[test]
    fn a_value_the_csv_has_is_never_replaced() {
        let web = page();
        let mut csv = csv_wood("Muiracatiara-rajada", web.scientific_name.as_str());
        csv.details.family = Some(BotanicalFamily::new("Csvaceae").unwrap());
        csv.details.collection_location = Some("Belém, PA".to_owned());
        csv.details.physical_properties.density.dry = density(9.99);
        csv.details.description.end_uses = Some("from the csv".to_owned());
        csv.details.description.tree = Attributes::new([("dap (cm)", "csv")]);
        csv.details.kiln_drying.duration = Some(Days::new(3.0).unwrap());
        csv.details.images.main = Some(WebUrl::new("https://csv.example/a.jpg").unwrap());
        csv.details.common_names = CommonNames::new(["Csv name"]);
        csv.details.workabilities.surface_finishing.planer = WorkabilityRating::Excellent;

        let wood = merge(vec![csv], vec![web.clone()]).woods.remove(0);

        let details = &wood.details;
        assert_eq!(details.family.as_ref().unwrap().as_str(), "Csvaceae");
        assert_eq!(details.collection_location.as_deref(), Some("Belém, PA"));
        assert_eq!(details.physical_properties.density.dry, density(9.99));
        assert_eq!(
            details.description.end_uses.as_deref(),
            Some("from the csv")
        );
        assert_eq!(details.description.tree.get("dap (cm)"), Some("csv"));
        assert_eq!(details.kiln_drying.duration.unwrap().value(), 3.0);
        assert_eq!(
            details.images.main.as_ref().unwrap().as_str(),
            "https://csv.example/a.jpg"
        );
        assert_eq!(
            details.workabilities.surface_finishing.planer,
            WorkabilityRating::Excellent
        );
        // What the CSV lacks still comes from the page, and the names are pooled.
        assert_eq!(details.images.tree, web.details.images.tree);
        assert!(
            details
                .common_names
                .as_slice()
                .contains(&"Csv name".to_owned())
        );
        for name in web.details.common_names.as_slice() {
            assert!(details.common_names.as_slice().contains(name), "{name}");
        }
        for (key, value) in web
            .details
            .description
            .tree
            .iter()
            .filter(|(key, _)| *key != "dap (cm)")
        {
            assert_eq!(details.description.tree.get(key), Some(value));
        }
    }

    #[test]
    fn species_are_matched_ignoring_case_and_spacing() {
        let web = page();
        let spaced = web
            .scientific_name
            .as_str()
            .to_uppercase()
            .replace(' ', "  ");
        let mut csv = csv_wood("Muiracatiara", "Genus species");
        csv.scientific_name = ScientificName::new(&spaced).unwrap();

        let merged = merge(vec![csv], vec![web]);

        assert_eq!((merged.enriched, merged.web_only), (1, 0));
    }

    #[test]
    fn only_the_first_lot_of_a_species_is_completed() {
        let web = page();
        let name = web.scientific_name.to_string();
        let lots = vec![
            csv_wood("First lot", &name),
            csv_wood("Second lot", &name),
            csv_wood("Other", "Other species"),
        ];

        let merged = merge(lots, vec![web]);

        assert_eq!(merged.enriched, 1);
        assert_eq!(merged.woods.len(), 3);
        assert!(!merged.woods[0].details.images.main.is_none());
        assert!(merged.woods[1].details.images.main.is_none());
        assert_eq!(merged.woods[1].name.as_str(), "Second lot");
        assert!(merged.woods[2].details.images.main.is_none());
    }

    #[test]
    fn web_only_species_follow_the_csv_in_name_order() {
        let mut zebrano = page();
        zebrano.scientific_name = ScientificName::new("Microberlinia zebrano").unwrap();
        let mut abarco = page();
        abarco.scientific_name = ScientificName::new("Cariniana abarco").unwrap();
        let csv = csv_wood("Aroeira", "Myracrodruon urundeuva");

        let merged = merge(vec![csv], vec![zebrano, abarco]);

        let names: Vec<_> = merged
            .woods
            .iter()
            .map(|w| w.scientific_name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Myracrodruon urundeuva",
                "Cariniana abarco",
                "Microberlinia zebrano"
            ]
        );
        assert_eq!((merged.enriched, merged.web_only), (0, 2));
    }

    #[test]
    fn an_empty_scrape_leaves_the_csv_as_it_is() {
        let csv = csv_wood("Aroeira", "Myracrodruon urundeuva");

        let merged = merge(vec![csv.clone()], Vec::new());

        assert_eq!(merged.woods.len(), 1);
        assert_eq!(merged.woods[0].details, csv.details);
        assert_eq!((merged.enriched, merged.web_only), (0, 0));
    }

    #[test]
    fn a_species_listed_twice_on_the_web_uses_its_first_page() {
        let first = page();
        let mut second = page();
        second.details.family = Some(BotanicalFamily::new("Other").unwrap());

        let merged = merge(Vec::new(), vec![first.clone(), second]);

        assert_eq!(merged.woods.len(), 1);
        assert_eq!(merged.woods[0].details.family, first.details.family);
    }
}
