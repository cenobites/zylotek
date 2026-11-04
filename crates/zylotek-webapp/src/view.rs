//! Turns contract types into display-ready strings, so templates stay free of logic.

use crate::i18n::Lang;
use zylotek_contracts::{
    assistant::{Property, Recommendation},
    woods::{WoodResponse, WorkabilityRating},
};

const MISSING: &str = "—";

pub struct Fact {
    pub label: &'static str,
    pub value: String,
}

pub struct Pair {
    pub key: String,
    pub value: String,
}

fn number(value: Option<f64>, decimals: usize, unit: &str, lang: Lang) -> String {
    match value.filter(|v| v.is_finite()) {
        Some(value) => lang.decimal(format!("{value:.decimals$} {unit}").trim_end().to_owned()),
        None => MISSING.to_owned(),
    }
}

/// The figures people compare woods on. Dry-wood values first, green as a fallback, the
/// same preference the assistant ranks by.
pub fn facts(wood: &WoodResponse, lang: Lang) -> Vec<Fact> {
    let text = lang.strings();
    let details = &wood.details;
    let mechanical = &details.mechanical_properties;
    let dry_or_green = |dry: Option<f64>, green: Option<f64>| dry.or(green);
    let density = &details.physical_properties.density;
    let surface = &details.workabilities.surface_finishing;
    let ratings: Vec<f64> = [
        surface.planer,
        surface.sandpaper,
        surface.vise,
        surface.drill,
    ]
    .into_iter()
    .filter_map(rating)
    .collect();
    let workability =
        (!ratings.is_empty()).then(|| ratings.iter().sum::<f64>() / ratings.len() as f64 * 100.0);

    vec![
        Fact {
            label: text.fact_family,
            value: details.family.clone().unwrap_or_else(|| MISSING.to_owned()),
        },
        Fact {
            label: text.fact_density,
            value: number(
                density.apparent.or(density.dry).or(density.basic),
                2,
                "g/cm³",
                lang,
            ),
        },
        Fact {
            label: text.fact_shrinkage,
            value: number(
                details.physical_properties.shrinkage.volumetric,
                1,
                "%",
                lang,
            ),
        },
        Fact {
            label: text.fact_hardness,
            value: number(
                dry_or_green(
                    mechanical.dry.janka_hardness.parallel_to_grain,
                    mechanical.green.janka_hardness.parallel_to_grain,
                ),
                0,
                "kgf",
                lang,
            ),
        },
        Fact {
            label: text.fact_bending,
            value: number(
                dry_or_green(
                    mechanical.dry.static_bending.modulus_of_rupture,
                    mechanical.green.static_bending.modulus_of_rupture,
                ),
                0,
                "kgf/cm²",
                lang,
            ),
        },
        Fact {
            label: text.fact_stiffness,
            value: number(
                dry_or_green(
                    mechanical.dry.static_bending.modulus_of_elasticity,
                    mechanical.green.static_bending.modulus_of_elasticity,
                ),
                0,
                "kgf/cm²",
                lang,
            ),
        },
        Fact {
            label: text.fact_workability,
            value: number(workability, 0, "%", lang),
        },
    ]
}

fn rating(rating: WorkabilityRating) -> Option<f64> {
    let step = match rating {
        WorkabilityRating::NotApplicable => return None,
        WorkabilityRating::VeryPoor => 0.0,
        WorkabilityRating::Poor => 1.0,
        WorkabilityRating::Fair => 2.0,
        WorkabilityRating::Easy => 3.0,
        WorkabilityRating::Good => 4.0,
        WorkabilityRating::VeryGood => 5.0,
        WorkabilityRating::Excellent => 6.0,
    };

    Some(step / 6.0)
}

pub struct WoodRow {
    pub id: String,
    pub name: String,
    pub scientific_name: String,
    pub family: String,
    pub hardness: String,
    pub density: String,
    /// Names the row's checkbox for screen readers.
    pub compare_label: String,
}

/// Where `facts` puts the figures a table row shows.
const FAMILY: usize = 0;
const DENSITY: usize = 1;
const HARDNESS: usize = 3;

pub fn row(wood: &WoodResponse, lang: Lang) -> WoodRow {
    let facts = facts(wood, lang);
    let value = |index: usize| {
        facts
            .get(index)
            .map_or_else(|| MISSING.to_owned(), |fact| fact.value.clone())
    };

    WoodRow {
        id: wood.id.clone(),
        name: wood.name.clone(),
        scientific_name: wood.scientific_name.clone(),
        family: value(FAMILY),
        hardness: value(HARDNESS),
        density: value(DENSITY),
        compare_label: lang.strings().compare_label(&wood.name),
    }
}

pub struct WoodView {
    pub id: String,
    pub name: String,
    pub scientific_name: String,
    pub common_names: String,
    /// "Collected in Belém, PA."
    pub location_note: Option<String>,
    pub image: Option<String>,
    pub source_url: Option<String>,
    pub facts: Vec<Fact>,
    pub tree: Vec<Pair>,
    pub characteristics: Vec<Pair>,
    pub end_uses: Option<String>,
    pub preservation: Option<String>,
    pub workability_notes: Option<String>,
}

/// Only web addresses become links or images: the data comes from a scraped site.
fn web_url(url: &Option<String>) -> Option<String> {
    url.as_ref()
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .cloned()
}

fn pairs(map: &std::collections::BTreeMap<String, String>) -> Vec<Pair> {
    map.iter()
        .map(|(key, value)| Pair {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

pub fn detail(wood: &WoodResponse, lang: Lang) -> WoodView {
    let details = &wood.details;
    let description = &details.description;

    WoodView {
        id: wood.id.clone(),
        name: wood.name.clone(),
        scientific_name: wood.scientific_name.clone(),
        common_names: details.common_names.join(", "),
        location_note: details
            .collection_location
            .as_deref()
            .map(|location| lang.strings().collected_in(location)),
        image: web_url(&details.images.main),
        source_url: web_url(
            &details
                .source
                .as_ref()
                .map(|source| source.page_url.clone()),
        ),
        facts: facts(wood, lang),
        tree: pairs(&description.tree),
        characteristics: pairs(&description.general_characteristics),
        end_uses: description.end_uses.clone(),
        preservation: description.preservation.clone(),
        workability_notes: description.workability_notes.clone(),
    }
}

pub struct ReasonView {
    pub label: &'static str,
    pub value: String,
    /// "better than 85% of woods".
    pub better_than: String,
}

pub struct CardView {
    pub id: String,
    pub name: String,
    pub scientific_name: String,
    pub percent: u32,
    pub reasons: Vec<ReasonView>,
}

pub fn label(property: Property, lang: Lang) -> &'static str {
    let text = lang.strings();
    match property {
        Property::Hardness => text.prop_hardness,
        Property::Strength => text.prop_strength,
        Property::Stiffness => text.prop_stiffness,
        Property::Stability => text.prop_stability,
        Property::Lightness => text.prop_lightness,
        Property::Workability => text.prop_workability,
    }
}

fn reading(property: Property, value: f64, lang: Lang) -> String {
    let text = lang.strings();
    let reading = match property {
        Property::Hardness => format!("{value:.0} kgf"),
        Property::Strength | Property::Stiffness => format!("{value:.0} kgf/cm²"),
        Property::Stability => format!("{value:.1}% {}", text.shrinkage_suffix),
        Property::Lightness => format!("{value:.2} g/cm³"),
        Property::Workability => format!("{:.0}% {}", value * 100.0, text.ease_suffix),
    };

    lang.decimal(reading)
}

fn percent(fraction: f64) -> u32 {
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u32
}

pub fn card(recommendation: &Recommendation, lang: Lang) -> CardView {
    CardView {
        id: recommendation.wood_id.clone(),
        name: recommendation.name.clone(),
        scientific_name: recommendation.scientific_name.clone(),
        percent: percent(recommendation.score),
        reasons: recommendation
            .reasons
            .iter()
            .map(|reason| ReasonView {
                label: label(reason.property, lang),
                value: reading(reason.property, reason.value, lang),
                better_than: lang.strings().better_than(percent(reason.fit)),
            })
            .collect(),
    }
}

#[cfg(test)]
pub mod testing {
    use zylotek_contracts::woods::WoodDetailsSchema;

    use super::*;

    pub fn wood(id: &str, name: &str) -> WoodResponse {
        WoodResponse {
            id: id.to_owned(),
            name: name.to_owned(),
            scientific_name: format!("{name} scientificus"),
            genus: name.to_owned(),
            details: WoodDetailsSchema::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use zylotek_contracts::assistant::Reason;

    use super::{testing::wood, *};

    fn value<'a>(facts: &'a [Fact], label: &str) -> &'a str {
        &facts.iter().find(|f| f.label == label).unwrap().value
    }

    #[test]
    fn a_wood_with_nothing_known_shows_dashes() {
        let facts = facts(&wood("1", "Bare"), Lang::En);

        assert_eq!(facts.len(), 7);
        assert!(facts.iter().all(|fact| fact.value == "—"));
    }

    #[test]
    fn known_figures_are_formatted_with_their_units() {
        let mut w = wood("1", "Full");
        w.details.family = Some("Meliaceae".into());
        w.details.physical_properties.density.dry = Some(0.5);
        w.details.physical_properties.shrinkage.volumetric = Some(12.34);
        w.details
            .mechanical_properties
            .green
            .janka_hardness
            .parallel_to_grain = Some(612.4);
        w.details
            .mechanical_properties
            .dry
            .static_bending
            .modulus_of_rupture = Some(930.0);
        w.details
            .mechanical_properties
            .dry
            .static_bending
            .modulus_of_elasticity = Some(f64::NAN);
        w.details.workabilities.surface_finishing.planer = WorkabilityRating::Excellent;
        w.details.workabilities.surface_finishing.vise = WorkabilityRating::Fair;
        w.details.workabilities.surface_finishing.drill = WorkabilityRating::NotApplicable;

        let facts = facts(&w, Lang::En);

        assert_eq!(value(&facts, "Family"), "Meliaceae");
        assert_eq!(value(&facts, "Density (apparent)"), "0.50 g/cm³");
        assert_eq!(value(&facts, "Volumetric shrinkage"), "12.3 %");
        assert_eq!(value(&facts, "Janka hardness"), "612 kgf");
        assert_eq!(value(&facts, "Bending strength"), "930 kgf/cm²");
        assert_eq!(value(&facts, "Stiffness (MOE)"), "—", "NaN is not shown");
        assert_eq!(value(&facts, "Ease of working"), "67 %");
    }

    #[test]
    fn every_rating_step_maps_onto_zero_to_one() {
        let steps: Vec<_> = [
            WorkabilityRating::VeryPoor,
            WorkabilityRating::Poor,
            WorkabilityRating::Fair,
            WorkabilityRating::Easy,
            WorkabilityRating::Good,
            WorkabilityRating::VeryGood,
            WorkabilityRating::Excellent,
        ]
        .into_iter()
        .map(|r| rating(r).unwrap())
        .collect();

        assert_eq!(steps[0], 0.0);
        assert_eq!(steps[6], 1.0);
        assert!(steps.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(rating(WorkabilityRating::NotApplicable), None);
    }

    #[test]
    fn a_table_row_picks_the_columns_a_list_shows() {
        let mut w = wood("7", "Ipê");
        w.details
            .mechanical_properties
            .dry
            .janka_hardness
            .parallel_to_grain = Some(1800.0);

        let row = row(&w, Lang::En);

        assert_eq!((row.id.as_str(), row.name.as_str()), ("7", "Ipê"));
        assert_eq!(row.hardness, "1800 kgf");
        assert_eq!((row.family.as_str(), row.density.as_str()), ("—", "—"));
    }

    #[test]
    fn the_detail_view_keeps_only_web_links() {
        let mut w = wood("1", "Cedro");
        w.details.common_names = vec!["Cedro-rosa".into(), "Cedro-vermelho".into()];
        w.details.images.main = Some("javascript:alert(1)".into());
        w.details.source = Some(zylotek_contracts::woods::SourceSchema {
            species_id: 3,
            page_url: "https://lpf.florestal.gov.br/x".into(),
        });
        w.details
            .description
            .tree
            .insert("Altura".into(), "30 m".into());
        w.details.description.end_uses = Some("Móveis".into());

        let view = detail(&w, Lang::En);

        assert_eq!(view.common_names, "Cedro-rosa, Cedro-vermelho");
        assert_eq!(view.image, None, "a javascript: URL is dropped");
        assert_eq!(
            view.source_url.as_deref(),
            Some("https://lpf.florestal.gov.br/x")
        );
        assert_eq!(view.tree[0].key, "Altura");
        assert_eq!(view.end_uses.as_deref(), Some("Móveis"));
        w.details.images.main = Some("http://example.org/a.jpg".into());
        assert_eq!(
            detail(&w, Lang::En).image.as_deref(),
            Some("http://example.org/a.jpg")
        );
    }

    #[test]
    fn a_recommendation_becomes_percentages_and_readable_figures() {
        let recommendation = Recommendation {
            wood_id: "1".into(),
            name: "Cedro".into(),
            scientific_name: "Cedrela".into(),
            score: 0.8449,
            reasons: Property::ALL
                .into_iter()
                .map(|property| Reason {
                    property,
                    value: 0.5,
                    fit: 1.7,
                })
                .collect(),
        };

        let card = card(&recommendation, Lang::En);

        assert_eq!(card.percent, 84);
        let readings: Vec<_> = card.reasons.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(
            readings,
            [
                "0 kgf",
                "0 kgf/cm²",
                "0 kgf/cm²",
                "0.5% shrinkage",
                "0.50 g/cm³",
                "50% ease"
            ]
        );
        assert!(
            card.reasons
                .iter()
                .all(|r| r.better_than == "better than 100% of woods"),
            "clamped"
        );
        assert_eq!(
            label(Property::Stability, Lang::En),
            "Dimensional stability"
        );
        assert_eq!(percent(-1.0), 0);
    }

    #[test]
    fn portuguese_figures_use_decimal_commas_and_translated_labels() {
        let mut w = wood("1", "Cedro");
        w.details.physical_properties.density.dry = Some(0.5);
        w.details.physical_properties.shrinkage.volumetric = Some(12.34);

        let facts = facts(&w, Lang::PtBr);
        let reading = |property, value| super::reading(property, value, Lang::PtBr);

        assert_eq!(value(&facts, "Densidade (aparente)"), "0,50 g/cm³");
        assert_eq!(value(&facts, "Retração volumétrica"), "12,3 %");
        assert_eq!(value(&facts, "Família"), "—");
        assert_eq!(reading(Property::Stability, 0.5), "0,5% de retração");
        assert_eq!(reading(Property::Workability, 0.5), "50% de facilidade");
        assert_eq!(reading(Property::Hardness, 900.0), "900 kgf");
        assert_eq!(label(Property::Lightness, Lang::PtBr), "Leveza");
        assert_eq!(detail_note(&w), None, "no location, no note");
        w.details.collection_location = Some("Belém, PA".into());
        assert_eq!(detail_note(&w).as_deref(), Some("Coletada em Belém, PA."));
        assert_eq!(row(&w, Lang::PtBr).compare_label, "Comparar Cedro");
    }

    fn detail_note(w: &WoodResponse) -> Option<String> {
        detail(w, Lang::PtBr).location_note
    }
}
