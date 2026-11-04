//! Turns contract types into display-ready strings, so templates stay free of logic.

use crate::{
    browse::{Density, Entry, Hardness, Query, Results, Sort, Stability, View},
    i18n::Lang,
};
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

pub struct WoodView {
    pub id: String,
    /// See [`grain`].
    pub grain: String,
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

/// A drawn stand-in for a photo: wood grain in the colour of the heartwood, its rings placed
/// from the id so every wood keeps a look of its own. Custom properties for a `style`
/// attribute; the `.grain` class in the stylesheet draws them.
pub fn grain(color: Option<&str>, seed: &str) -> String {
    let color = color.map(crate::browse::fold).unwrap_or_default();
    let tone = [
        ("marr", "#8b5a35"),
        ("amarel", "#c79a45"),
        ("branc", "#dcc8a6"),
        ("bege", "#d2b48c"),
        ("cinz", "#948a7c"),
        ("ros", "#c48a74"),
        ("vermel", "#9a432c"),
        ("oliv", "#86804a"),
        ("pret", "#3d3029"),
        ("rox", "#6c3d4c"),
    ]
    .iter()
    .find(|(name, _)| color.starts_with(name))
    .map_or("#a87449", |(_, hex)| hex);
    // FNV-1a: stable across runs and machines, unlike the standard hasher.
    let hash = seed.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });

    format!(
        "--wood:{tone};--gx:{}%;--gy:{}%;--ring:{}px",
        10 + hash % 81,
        115 + (hash >> 8) % 70,
        18 + (hash >> 16) % 18
    )
}

/// Only web addresses become links or images: the data comes from a scraped site.
pub fn web_url(url: &Option<String>) -> Option<String> {
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
        grain: grain(
            description.heartwood_color_classification.as_deref(),
            &wood.id,
        ),
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

// ---- browsing -------------------------------------------------------------------------

/// A short label on a wood: its hardness band, colour, place... Following it filters by it.
pub struct TagView {
    pub label: String,
    pub href: String,
    /// What the tag is about, for styling.
    pub kind: &'static str,
}

/// A wood as the browse page shows it, as a card or as a row.
pub struct ItemView {
    pub id: String,
    /// See [`grain`].
    pub grain: String,
    pub name: String,
    pub scientific_name: String,
    pub common_names: String,
    pub family: String,
    pub image: Option<String>,
    pub tags: Vec<TagView>,
    pub hardness: String,
    pub density: String,
    pub shrinkage: String,
    /// Names the compare checkbox for screen readers.
    pub compare_label: String,
}

fn add<T: PartialEq>(list: &mut Vec<T>, item: T) {
    if !list.contains(&item) {
        list.push(item);
    }
}

pub fn item(entry: &Entry, query: &Query, lang: Lang) -> ItemView {
    let text = lang.strings();
    let wood = entry.wood;
    let tag = |kind: &'static str, label: String, href: String| TagView { label, href, kind };
    let mut tags = Vec::new();
    if let Some(class) = entry.hardness_class() {
        tags.push(tag(
            "hardness",
            text.hardness_classes[class.index()].to_owned(),
            query.link(|q| add(&mut q.hardness, class)),
        ));
    }
    if let Some(class) = entry.density_class() {
        tags.push(tag(
            "density",
            text.density_classes[class.index()].to_owned(),
            query.link(|q| add(&mut q.density, class)),
        ));
    }
    if let Some(class) = entry.stability_class() {
        tags.push(tag(
            "stability",
            text.stability_classes[class.index()].to_owned(),
            query.link(|q| add(&mut q.stability, class)),
        ));
    }
    if let Some(color) = &entry.color {
        tags.push(tag(
            "color",
            text.color_name(color),
            query.link(|q| add(&mut q.colors, color.clone())),
        ));
    }
    if let Some(region) = &entry.region {
        tags.push(tag(
            "region",
            region.clone(),
            query.link(|q| add(&mut q.regions, region.clone())),
        ));
    }

    ItemView {
        id: wood.id.clone(),
        grain: grain(entry.color.as_deref(), &wood.id),
        name: wood.name.clone(),
        scientific_name: wood.scientific_name.clone(),
        common_names: wood
            .details
            .common_names
            .iter()
            .filter(|name| **name != wood.name)
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        family: entry.family.clone().unwrap_or_else(|| MISSING.to_owned()),
        image: entry.image.clone(),
        tags,
        hardness: number(entry.hardness, 0, "kgf", lang),
        density: number(entry.density, 2, "g/cm³", lang),
        shrinkage: number(entry.shrinkage, 1, "%", lang),
        compare_label: text.compare_label(&wood.name),
    }
}

pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
    pub selected: bool,
}

pub struct FacetOption {
    pub value: String,
    pub label: String,
    pub count: usize,
    pub checked: bool,
}

/// One group of checkboxes in the filter panel.
pub struct FacetGroup {
    /// The query parameter its checkboxes send.
    pub name: &'static str,
    pub label: &'static str,
    pub options: Vec<FacetOption>,
    pub open: bool,
}

/// An active filter, with the address that takes it off.
pub struct Chip {
    pub label: String,
    pub href: String,
}

/// A column title in the list view; most sort the list.
pub struct Header {
    pub label: &'static str,
    pub href: Option<String>,
    pub arrow: &'static str,
}

/// Everything the browse page shows.
pub struct BrowseView {
    pub text: String,
    pub list: bool,
    pub summary: String,
    pub sorts: Vec<Choice>,
    pub groups: Vec<FacetGroup>,
    pub hardness_min: String,
    pub hardness_max: String,
    pub density_min: String,
    pub density_max: String,
    pub with_image: bool,
    pub image_count: usize,
    pub active: Vec<Chip>,
    pub clear: Option<String>,
    pub cards_href: String,
    pub list_href: String,
    pub headers: Vec<Header>,
    pub items: Vec<ItemView>,
    pub previous: Option<String>,
    pub next: Option<String>,
    pub page_label: String,
}

fn bound(value: Option<f64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

pub fn browse(total: usize, query: &Query, results: &Results, lang: Lang) -> BrowseView {
    let text = lang.strings();
    let facets = &results.facets;

    let values = |counts: &[(String, usize)],
                  chosen: &[String],
                  label: &dyn Fn(&str) -> String|
     -> Vec<FacetOption> {
        counts
            .iter()
            .map(|(value, count)| FacetOption {
                value: value.clone(),
                label: label(value),
                count: *count,
                checked: chosen.contains(value),
            })
            .collect()
    };
    let classes =
        |counts: &[usize], labels: &[&str], keys: Vec<&'static str>, chosen: Vec<bool>| {
            keys.into_iter()
                .zip(counts)
                .zip(labels)
                .zip(chosen)
                .map(|(((key, count), label), checked)| FacetOption {
                    value: key.to_owned(),
                    label: (*label).to_owned(),
                    count: *count,
                    checked,
                })
                .collect::<Vec<_>>()
        };
    let group = |name, label, options: Vec<FacetOption>, always_open| FacetGroup {
        name,
        label,
        open: always_open || options.iter().any(|option| option.checked),
        options,
    };
    let groups = vec![
        group(
            "hardness",
            text.facet_hardness,
            classes(
                &facets.hardness,
                &text.hardness_classes,
                Hardness::ALL.iter().map(|c| c.key()).collect(),
                Hardness::ALL
                    .iter()
                    .map(|c| query.hardness.contains(c))
                    .collect(),
            ),
            true,
        ),
        group(
            "density",
            text.facet_density,
            classes(
                &facets.density,
                &text.density_classes,
                Density::ALL.iter().map(|c| c.key()).collect(),
                Density::ALL
                    .iter()
                    .map(|c| query.density.contains(c))
                    .collect(),
            ),
            true,
        ),
        group(
            "stability",
            text.facet_stability,
            classes(
                &facets.stability,
                &text.stability_classes,
                Stability::ALL.iter().map(|c| c.key()).collect(),
                Stability::ALL
                    .iter()
                    .map(|c| query.stability.contains(c))
                    .collect(),
            ),
            true,
        ),
        group(
            "color",
            text.facet_color,
            values(&facets.colors, &query.colors, &|c| text.color_name(c)),
            true,
        ),
        group(
            "region",
            text.facet_region,
            values(&facets.regions, &query.regions, &|r| r.to_owned()),
            false,
        ),
        group(
            "family",
            text.facet_family,
            values(&facets.families, &query.families, &|f| f.to_owned()),
            false,
        ),
    ];

    let chip = |label: String, href: String| Chip { label, href };
    let mut active = Vec::new();
    if !query.text.is_empty() {
        active.push(chip(
            text.search_chip(&query.text),
            query.link(|q| q.text.clear()),
        ));
    }
    for family in &query.families {
        active.push(chip(
            family.clone(),
            query.link(|q| q.families.retain(|f| f != family)),
        ));
    }
    for color in &query.colors {
        active.push(chip(
            text.color_name(color),
            query.link(|q| q.colors.retain(|c| c != color)),
        ));
    }
    for region in &query.regions {
        active.push(chip(
            region.clone(),
            query.link(|q| q.regions.retain(|r| r != region)),
        ));
    }
    for class in &query.hardness {
        active.push(chip(
            text.hardness_classes[class.index()].to_owned(),
            query.link(|q| q.hardness.retain(|c| c != class)),
        ));
    }
    for class in &query.density {
        active.push(chip(
            text.density_classes[class.index()].to_owned(),
            query.link(|q| q.density.retain(|c| c != class)),
        ));
    }
    for class in &query.stability {
        active.push(chip(
            text.stability_classes[class.index()].to_owned(),
            query.link(|q| q.stability.retain(|c| c != class)),
        ));
    }
    if query.hardness_min.is_some() || query.hardness_max.is_some() {
        active.push(chip(
            text.range_chip("Janka", query.hardness_min, query.hardness_max),
            query.link(|q| (q.hardness_min, q.hardness_max) = (None, None)),
        ));
    }
    if query.density_min.is_some() || query.density_max.is_some() {
        active.push(chip(
            text.range_chip(
                text.density_range.split(" (").next().unwrap_or_default(),
                query.density_min,
                query.density_max,
            ),
            query.link(|q| (q.density_min, q.density_max) = (None, None)),
        ));
    }
    if query.with_image {
        active.push(chip(
            text.only_with_photo.to_owned(),
            query.link(|q| q.with_image = false),
        ));
    }

    // Column titles that sort: each click moves on to the next order, or back to the first.
    let column = |label: &'static str, orders: &[(Sort, &'static str)]| {
        let at = orders.iter().position(|(sort, _)| *sort == query.sort());
        let next = orders[at.map_or(0, |at| (at + 1) % orders.len())].0;

        Header {
            label,
            href: Some(query.link(|q| q.sort = Some(next))),
            arrow: at.map_or("", |at| orders[at].1),
        }
    };
    let plain = |label| Header {
        label,
        href: None,
        arrow: "",
    };
    let headers = vec![
        column(text.col_name, &[(Sort::Name, "▲"), (Sort::NameDesc, "▼")]),
        plain(text.col_scientific_name),
        plain(text.col_family),
        column(
            text.col_hardness,
            &[(Sort::Hardest, "▼"), (Sort::Softest, "▲")],
        ),
        column(
            text.col_density,
            &[(Sort::Heaviest, "▼"), (Sort::Lightest, "▲")],
        ),
        column(text.col_shrinkage, &[(Sort::Stablest, "▲")]),
    ];

    BrowseView {
        text: query.text.clone(),
        list: query.view() == View::List,
        summary: text.results_found(results.found, total),
        sorts: Sort::ALL
            .iter()
            .filter(|sort| **sort != Sort::Relevance || !query.text.is_empty())
            .map(|sort| Choice {
                value: sort.key(),
                label: text.sort_labels[sort.index()],
                selected: *sort == query.sort(),
            })
            .collect(),
        groups,
        hardness_min: bound(query.hardness_min),
        hardness_max: bound(query.hardness_max),
        density_min: bound(query.density_min),
        density_max: bound(query.density_max),
        with_image: query.with_image,
        image_count: facets.with_image,
        active,
        clear: (!query.is_unfiltered()).then(|| {
            query.link(|q| {
                *q = Query {
                    sort: q.sort,
                    view: q.view,
                    ..Query::parse(None)
                }
            })
        }),
        cards_href: query.link(|q| {
            q.view = None;
            q.page = results.page;
        }),
        list_href: query.link(|q| {
            q.view = Some(View::List);
            q.page = results.page;
        }),
        headers,
        items: results
            .shown
            .iter()
            .map(|entry| item(entry, query, lang))
            .collect(),
        previous: (results.page > 1).then(|| query.link(|q| q.page = results.page - 1)),
        next: (results.page < results.pages).then(|| query.link(|q| q.page = results.page + 1)),
        page_label: text.page_of(results.page, results.pages),
    }
}

// ---- comparing ------------------------------------------------------------------------

pub struct CompareColumn {
    pub item: ItemView,
    /// The comparison without this wood; only while more than the least are left.
    pub remove: Option<String>,
}

pub struct CompareCell {
    pub text: String,
    /// The best figure of the row, when the woods differ.
    pub best: bool,
    /// The figure against the row's highest, for a bar.
    pub percent: Option<u32>,
}

pub struct CompareRow {
    pub label: &'static str,
    pub cells: Vec<CompareCell>,
}

/// Which end of a row is the good one.
#[derive(Clone, Copy)]
enum Better {
    Higher,
    Lower,
    Neither,
}

pub fn compare_href(ids: &[&str]) -> String {
    let pairs: Vec<String> = ids.iter().map(|id| format!("id={id}")).collect();

    format!("/compare?{}", pairs.join("&"))
}

pub fn comparison(
    woods: &[WoodResponse],
    removable: bool,
    lang: Lang,
) -> (Vec<CompareColumn>, Vec<CompareRow>) {
    let text = lang.strings();
    let entries: Vec<Entry> = woods.iter().map(Entry::new).collect();
    let nothing = Query::parse(None);
    let columns = entries
        .iter()
        .map(|entry| CompareColumn {
            item: item(entry, &nothing, lang),
            remove: removable.then(|| {
                let kept: Vec<&str> = woods
                    .iter()
                    .map(|w| w.id.as_str())
                    .filter(|id| *id != entry.wood.id)
                    .collect();

                compare_href(&kept)
            }),
        })
        .collect();

    let words = |label, value: &dyn Fn(&Entry) -> Option<String>| CompareRow {
        label,
        cells: entries
            .iter()
            .map(|entry| CompareCell {
                text: value(entry).unwrap_or_else(|| MISSING.to_owned()),
                best: false,
                percent: None,
            })
            .collect(),
    };
    let figures = |label, value: &dyn Fn(&Entry) -> Option<f64>, decimals, unit, better: Better| {
        let values: Vec<Option<f64>> = entries.iter().map(value).collect();
        let present = || values.iter().flatten().copied();
        let highest = present().fold(f64::MIN, f64::max);
        let lowest = present().fold(f64::MAX, f64::min);
        let target = match better {
            Better::Higher => Some(highest),
            Better::Lower => Some(lowest),
            Better::Neither => None,
        };

        CompareRow {
            label,
            cells: values
                .iter()
                .map(|value| CompareCell {
                    text: number(*value, decimals, unit, lang),
                    // Only when the woods differ is one of them "the best".
                    best: present().count() > 1
                        && highest != lowest
                        && value.is_some_and(|v| Some(v) == target),
                    percent: value
                        .filter(|_| highest > 0.0)
                        .map(|v| percent(v / highest)),
                })
                .collect(),
        }
    };

    let rows = vec![
        words(text.fact_family, &|e| e.family.clone()),
        words(text.fact_color, &|e| {
            e.color.as_deref().map(|c| text.color_name(c))
        }),
        words(text.fact_region, &|e| {
            e.wood.details.collection_location.clone()
        }),
        figures(
            text.fact_hardness,
            &|e| e.hardness,
            0,
            "kgf",
            Better::Higher,
        ),
        figures(
            text.fact_density,
            &|e| e.density,
            2,
            "g/cm³",
            Better::Neither,
        ),
        figures(text.fact_shrinkage, &|e| e.shrinkage, 1, "%", Better::Lower),
        figures(
            text.fact_bending,
            &|e| e.bending,
            0,
            "kgf/cm²",
            Better::Higher,
        ),
        figures(
            text.fact_stiffness,
            &|e| e.stiffness,
            0,
            "kgf/cm²",
            Better::Higher,
        ),
    ];

    (columns, rows)
}

pub struct ReasonView {
    pub label: &'static str,
    pub value: String,
    /// "better than 85% of woods".
    pub better_than: String,
}

pub struct CardView {
    pub id: String,
    /// See [`grain`]; a recommendation carries no colour, so only the rings vary.
    pub grain: String,
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
        grain: grain(None, &recommendation.wood_id),
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
    fn the_grain_takes_the_heartwood_colour_and_stays_the_same_for_a_wood() {
        assert!(grain(Some("Vermelha"), "a").starts_with("--wood:#9a432c;"));
        assert!(grain(Some("marrom-escuro"), "a").starts_with("--wood:#8b5a35;"));
        assert!(grain(Some("lilás"), "a").starts_with("--wood:#a87449;"));
        assert!(grain(None, "a").starts_with("--wood:#a87449;"));
        assert_eq!(grain(None, "ipe"), grain(None, "ipe"));
        assert_ne!(grain(None, "ipe"), grain(None, "cedro"));
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
    }

    fn detail_note(w: &WoodResponse) -> Option<String> {
        detail(w, Lang::PtBr).location_note
    }
}
