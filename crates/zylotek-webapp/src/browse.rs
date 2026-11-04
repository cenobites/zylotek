//! Browsing the catalogue: full-text search, filters, sorting and paging, over every wood held
//! in memory. The query lives in the URL (`/woods?q=ipe&hardness=hard&view=list`), so a search
//! can be bookmarked, shared and stepped back through, and the page needs no script to work.

use std::{cmp::Ordering, collections::BTreeMap};

use zylotek_contracts::woods::WoodResponse;

use crate::view::web_url;

pub const PAGE_SIZE: usize = 24;
const MAX_TEXT_CHARS: usize = 200;
const MAX_TERMS: usize = 8;
/// Values accepted per filter, so a hostile URL cannot make the page do unbounded work.
const MAX_VALUES: usize = 12;

/// Declares an enum whose variants have a short name for URLs.
macro_rules! keyed {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $key:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The name used in URLs.
            pub fn key(self) -> &'static str {
                match self { $($name::$variant => $key),+ }
            }

            pub fn from_key(key: &str) -> Option<Self> {
                match key { $($key => Some($name::$variant),)+ _ => None }
            }

            /// The place in `ALL`, for picking the matching translation.
            pub fn index(self) -> usize {
                self as usize
            }
        }
    };
}

keyed!(
    /// Janka hardness bands, in kgf.
    Hardness { Soft => "soft", Medium => "medium", Hard => "hard", VeryHard => "very-hard" }
);
keyed!(
    /// Density bands, in g/cm³.
    Density { Light => "light", Medium => "medium", Heavy => "heavy" }
);
keyed!(
    /// How much the wood moves with humidity, by volumetric shrinkage.
    Stability { Stable => "stable", Moderate => "moderate", Unstable => "unstable" }
);
keyed!(Sort {
    Relevance => "relevance",
    Name => "name",
    NameDesc => "name-desc",
    Hardest => "hardest",
    Softest => "softest",
    Heaviest => "heaviest",
    Lightest => "lightest",
    Stablest => "stablest",
});
keyed!(View { Cards => "cards", List => "list" });

impl Hardness {
    pub fn of(janka_kgf: f64) -> Self {
        match janka_kgf {
            h if h < 500.0 => Hardness::Soft,
            h if h < 1000.0 => Hardness::Medium,
            h if h < 1500.0 => Hardness::Hard,
            _ => Hardness::VeryHard,
        }
    }
}

impl Density {
    pub fn of(grams_per_cm3: f64) -> Self {
        match grams_per_cm3 {
            d if d < 0.55 => Density::Light,
            d if d < 0.80 => Density::Medium,
            _ => Density::Heavy,
        }
    }
}

impl Stability {
    pub fn of(volumetric_shrinkage_percent: f64) -> Self {
        match volumetric_shrinkage_percent {
            s if s < 10.0 => Stability::Stable,
            s if s < 15.0 => Stability::Moderate,
            _ => Stability::Unstable,
        }
    }
}

/// Lowercase without accents, so "ipe" finds "Ipê" and "acai" finds "Açaí".
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// One wood, with the figures and words browsing needs worked out once.
pub struct Entry<'a> {
    pub wood: &'a WoodResponse,
    /// Janka hardness in kgf, dry wood first.
    pub hardness: Option<f64>,
    /// g/cm³.
    pub density: Option<f64>,
    /// Volumetric shrinkage, %.
    pub shrinkage: Option<f64>,
    pub bending: Option<f64>,
    pub stiffness: Option<f64>,
    pub family: Option<String>,
    /// The heartwood colour, lowercase.
    pub color: Option<String>,
    /// The state in the collection site ("Santarém, PA" → "PA").
    pub region: Option<String>,
    pub image: Option<String>,
    name_key: String,
    names_key: String,
    search: String,
}

impl<'a> Entry<'a> {
    pub fn new(wood: &'a WoodResponse) -> Self {
        let details = &wood.details;
        let mechanical = &details.mechanical_properties;
        let density = &details.physical_properties.density;
        let description = &details.description;
        let finite = |value: Option<f64>| value.filter(|v| v.is_finite());
        let dry_or_green = |dry: Option<f64>, green: Option<f64>| finite(dry.or(green));
        let text = |value: &Option<String>| value.clone().filter(|v| !v.trim().is_empty());

        let color = text(&description.heartwood_color_classification)
            .map(|color| color.trim().to_lowercase());
        let region = details.collection_location.as_deref().and_then(|place| {
            let state = place.rsplit(',').next()?.trim();
            (state.len() == 2 && state.bytes().all(|b| b.is_ascii_alphabetic()))
                .then(|| state.to_ascii_uppercase())
        });

        let names_key = fold(&format!(
            "{} {} {} {}",
            wood.scientific_name,
            wood.genus,
            details.common_names.join(" "),
            details.family.as_deref().unwrap_or_default()
        ));
        let mut search = format!(
            "{} {} {} {}",
            fold(&wood.name),
            names_key,
            color.as_deref().map(fold).unwrap_or_default(),
            fold(details.collection_location.as_deref().unwrap_or_default()),
        );
        for part in [
            &description.end_uses,
            &description.preservation,
            &description.workability_notes,
        ]
        .into_iter()
        .flatten()
        {
            search.push(' ');
            search.push_str(&fold(part));
        }
        for (key, value) in description
            .general_characteristics
            .iter()
            .chain(description.tree.iter())
        {
            search.push(' ');
            search.push_str(&fold(key));
            search.push(' ');
            search.push_str(&fold(value));
        }

        Self {
            wood,
            hardness: dry_or_green(
                mechanical.dry.janka_hardness.parallel_to_grain,
                mechanical.green.janka_hardness.parallel_to_grain,
            ),
            density: finite(density.apparent.or(density.dry).or(density.basic)),
            shrinkage: finite(details.physical_properties.shrinkage.volumetric),
            bending: dry_or_green(
                mechanical.dry.static_bending.modulus_of_rupture,
                mechanical.green.static_bending.modulus_of_rupture,
            ),
            stiffness: dry_or_green(
                mechanical.dry.static_bending.modulus_of_elasticity,
                mechanical.green.static_bending.modulus_of_elasticity,
            ),
            family: text(&details.family),
            color,
            region,
            image: web_url(&details.images.main),
            name_key: fold(&wood.name),
            names_key,
            search,
        }
    }

    pub fn hardness_class(&self) -> Option<Hardness> {
        self.hardness.map(Hardness::of)
    }

    pub fn density_class(&self) -> Option<Density> {
        self.density.map(Density::of)
    }

    pub fn stability_class(&self) -> Option<Stability> {
        self.shrinkage.map(Stability::of)
    }
}

/// What the visitor asked for, as read from the URL.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    pub text: String,
    pub families: Vec<String>,
    pub colors: Vec<String>,
    pub regions: Vec<String>,
    pub hardness: Vec<Hardness>,
    pub density: Vec<Density>,
    pub stability: Vec<Stability>,
    pub hardness_min: Option<f64>,
    pub hardness_max: Option<f64>,
    pub density_min: Option<f64>,
    pub density_max: Option<f64>,
    pub with_image: bool,
    /// `None` is the default: by relevance while searching, by name otherwise.
    pub sort: Option<Sort>,
    pub view: Option<View>,
    /// From 1; asking past the last page shows the last.
    pub page: usize,
}

fn push_unique<T: PartialEq>(list: &mut Vec<T>, item: T) {
    if list.len() < MAX_VALUES && !list.contains(&item) {
        list.push(item);
    }
}

fn number(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0)
}

impl Query {
    /// Reads a query string. What it does not understand is ignored: a page that opens beats
    /// an error about a stale bookmark.
    pub fn parse(raw: Option<&str>) -> Self {
        let mut query = Query {
            page: 1,
            ..Query::default()
        };
        for (key, value) in form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match key.as_ref() {
                "q" => query.text = value.chars().take(MAX_TEXT_CHARS).collect(),
                "family" => push_unique(&mut query.families, value.to_owned()),
                "color" => push_unique(&mut query.colors, value.to_lowercase()),
                "region" => push_unique(&mut query.regions, value.to_ascii_uppercase()),
                "hardness" => {
                    if let Some(class) = Hardness::from_key(value) {
                        push_unique(&mut query.hardness, class);
                    }
                }
                "density" => {
                    if let Some(class) = Density::from_key(value) {
                        push_unique(&mut query.density, class);
                    }
                }
                "stability" => {
                    if let Some(class) = Stability::from_key(value) {
                        push_unique(&mut query.stability, class);
                    }
                }
                "hardness_min" => query.hardness_min = number(value),
                "hardness_max" => query.hardness_max = number(value),
                "density_min" => query.density_min = number(value),
                "density_max" => query.density_max = number(value),
                "image" => query.with_image = value == "1",
                "sort" => query.sort = Sort::from_key(value),
                "view" => query.view = View::from_key(value),
                "page" => query.page = value.parse().ok().filter(|p| *p >= 1).unwrap_or(1),
                _ => {}
            }
        }

        query
    }

    pub fn sort(&self) -> Sort {
        self.sort.unwrap_or(if self.text.is_empty() {
            Sort::Name
        } else {
            Sort::Relevance
        })
    }

    pub fn view(&self) -> View {
        self.view.unwrap_or(View::Cards)
    }

    /// True when nothing narrows the list.
    pub fn is_unfiltered(&self) -> bool {
        let unfiltered = Query {
            sort: self.sort,
            view: self.view,
            page: self.page,
            ..Query::parse(None)
        };

        *self == unfiltered
    }

    /// The address that asks for this query. What is at its default is left out.
    pub fn url(&self) -> String {
        let mut pairs = form_urlencoded::Serializer::new(String::new());
        if !self.text.is_empty() {
            pairs.append_pair("q", &self.text);
        }
        for family in &self.families {
            pairs.append_pair("family", family);
        }
        for color in &self.colors {
            pairs.append_pair("color", color);
        }
        for region in &self.regions {
            pairs.append_pair("region", region);
        }
        for class in &self.hardness {
            pairs.append_pair("hardness", class.key());
        }
        for class in &self.density {
            pairs.append_pair("density", class.key());
        }
        for class in &self.stability {
            pairs.append_pair("stability", class.key());
        }
        for (name, value) in [
            ("hardness_min", self.hardness_min),
            ("hardness_max", self.hardness_max),
            ("density_min", self.density_min),
            ("density_max", self.density_max),
        ] {
            if let Some(value) = value {
                pairs.append_pair(name, &value.to_string());
            }
        }
        if self.with_image {
            pairs.append_pair("image", "1");
        }
        if let Some(sort) = self.sort {
            pairs.append_pair("sort", sort.key());
        }
        if self.view == Some(View::List) {
            pairs.append_pair("view", View::List.key());
        }
        if self.page > 1 {
            pairs.append_pair("page", &self.page.to_string());
        }
        let pairs = pairs.finish();

        if pairs.is_empty() {
            "/woods".to_owned()
        } else {
            format!("/woods?{pairs}")
        }
    }

    /// The address of this query with a change made, back on the first page unless the change
    /// moves it: filtering from page 3 would otherwise land past the end.
    pub fn link(&self, change: impl FnOnce(&mut Query)) -> String {
        let mut changed = self.clone();
        changed.page = 1;
        change(&mut changed);

        changed.url()
    }

    fn terms(&self) -> Vec<String> {
        fold(&self.text)
            .split_whitespace()
            .take(MAX_TERMS)
            .map(str::to_owned)
            .collect()
    }

    /// Whether a wood passes everything asked, except the facet named in `skip`; counting the
    /// options of a facet needs the list as it would be without that facet's own choice.
    fn accepts(&self, entry: &Entry, terms: &[String], skip: Option<Facet>) -> bool {
        let applies = |facet: Facet| skip != Some(facet);
        let one_of = |nothing_chosen: bool, found: bool| nothing_chosen || found;

        terms
            .iter()
            .all(|term| entry.search.contains(term.as_str()))
            && within(entry.hardness, self.hardness_min, self.hardness_max)
            && within(entry.density, self.density_min, self.density_max)
            && (!applies(Facet::Family)
                || one_of(
                    self.families.is_empty(),
                    entry
                        .family
                        .as_ref()
                        .is_some_and(|f| self.families.contains(f)),
                ))
            && (!applies(Facet::Color)
                || one_of(
                    self.colors.is_empty(),
                    entry
                        .color
                        .as_ref()
                        .is_some_and(|c| self.colors.contains(c)),
                ))
            && (!applies(Facet::Region)
                || one_of(
                    self.regions.is_empty(),
                    entry
                        .region
                        .as_ref()
                        .is_some_and(|r| self.regions.contains(r)),
                ))
            && (!applies(Facet::Hardness)
                || one_of(
                    self.hardness.is_empty(),
                    entry
                        .hardness_class()
                        .is_some_and(|c| self.hardness.contains(&c)),
                ))
            && (!applies(Facet::Density)
                || one_of(
                    self.density.is_empty(),
                    entry
                        .density_class()
                        .is_some_and(|c| self.density.contains(&c)),
                ))
            && (!applies(Facet::Stability)
                || one_of(
                    self.stability.is_empty(),
                    entry
                        .stability_class()
                        .is_some_and(|c| self.stability.contains(&c)),
                ))
            && (!applies(Facet::Image) || !self.with_image || entry.image.is_some())
    }

    /// The woods that match, in order, one page of them, and the counts for every facet.
    pub fn search<'e, 'a>(&self, entries: &'e [Entry<'a>]) -> Results<'e, 'a> {
        let terms = self.terms();
        let mut matched: Vec<&Entry> = entries
            .iter()
            .filter(|entry| self.accepts(entry, &terms, None))
            .collect();
        let by_name = |a: &Entry, b: &Entry| a.name_key.cmp(&b.name_key);
        matched.sort_by(|a, b| {
            let order = match self.sort() {
                Sort::Relevance => relevance(b, &terms).cmp(&relevance(a, &terms)),
                Sort::Name => Ordering::Equal,
                Sort::NameDesc => by_name(b, a),
                Sort::Hardest => by_number(a.hardness, b.hardness, true),
                Sort::Softest => by_number(a.hardness, b.hardness, false),
                Sort::Heaviest => by_number(a.density, b.density, true),
                Sort::Lightest => by_number(a.density, b.density, false),
                Sort::Stablest => by_number(a.shrinkage, b.shrinkage, false),
            };

            order.then_with(|| by_name(a, b))
        });

        let pages = matched.len().div_ceil(PAGE_SIZE).max(1);
        let page = self.page.clamp(1, pages);
        let found = matched.len();
        let shown = matched
            .into_iter()
            .skip((page - 1) * PAGE_SIZE)
            .take(PAGE_SIZE)
            .collect();

        Results {
            found,
            page,
            pages,
            shown,
            facets: self.facets(entries, &terms),
        }
    }

    fn facets(&self, entries: &[Entry], terms: &[String]) -> Facets {
        let counted = |facet: Facet| {
            entries
                .iter()
                .filter(move |entry| self.accepts(entry, terms, Some(facet)))
        };
        let tally = |facet: Facet, value: fn(&Entry) -> Option<String>, selected: &[String]| {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for entry in counted(facet) {
                if let Some(value) = value(entry) {
                    *counts.entry(value).or_default() += 1;
                }
            }
            // A choice that matches nothing now must stay on the list, or it could not be undone.
            for value in selected {
                counts.entry(value.clone()).or_default();
            }
            let mut counts: Vec<(String, usize)> = counts.into_iter().collect();
            counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

            counts
        };
        let classes = |facet: Facet, class: fn(&Entry) -> Option<usize>, count: usize| {
            let mut counts = vec![0; count];
            for entry in counted(facet) {
                if let Some(index) = class(entry) {
                    counts[index] += 1;
                }
            }

            counts
        };

        Facets {
            families: tally(Facet::Family, |e| e.family.clone(), &self.families),
            colors: tally(Facet::Color, |e| e.color.clone(), &self.colors),
            regions: tally(Facet::Region, |e| e.region.clone(), &self.regions),
            hardness: classes(
                Facet::Hardness,
                |e| e.hardness_class().map(Hardness::index),
                Hardness::ALL.len(),
            ),
            density: classes(
                Facet::Density,
                |e| e.density_class().map(Density::index),
                Density::ALL.len(),
            ),
            stability: classes(
                Facet::Stability,
                |e| e.stability_class().map(Stability::index),
                Stability::ALL.len(),
            ),
            with_image: counted(Facet::Image)
                .filter(|entry| entry.image.is_some())
                .count(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Facet {
    Family,
    Color,
    Region,
    Hardness,
    Density,
    Stability,
    Image,
}

fn within(value: Option<f64>, min: Option<f64>, max: Option<f64>) -> bool {
    if min.is_none() && max.is_none() {
        return true;
    }

    value.is_some_and(|v| min.is_none_or(|m| v >= m) && max.is_none_or(|m| v <= m))
}

/// Numbers in order, woods without the figure always last.
fn by_number(a: Option<f64>, b: Option<f64>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) if descending => b.total_cmp(&a),
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// A name beats a scientific or common name, which beats a mention somewhere in the text.
fn relevance(entry: &Entry, terms: &[String]) -> usize {
    terms
        .iter()
        .map(|term| {
            if entry.name_key.starts_with(term.as_str()) {
                5
            } else if entry.name_key.contains(term.as_str()) {
                4
            } else if entry.names_key.contains(term.as_str()) {
                3
            } else {
                1
            }
        })
        .sum()
}

pub struct Results<'e, 'a> {
    /// How many woods match, over all pages.
    pub found: usize,
    pub page: usize,
    pub pages: usize,
    pub shown: Vec<&'e Entry<'a>>,
    pub facets: Facets,
}

/// How many woods each choice would leave, given the other choices already made.
pub struct Facets {
    /// Most common first.
    pub families: Vec<(String, usize)>,
    pub colors: Vec<(String, usize)>,
    pub regions: Vec<(String, usize)>,
    /// In the order of `Hardness::ALL`, and so on.
    pub hardness: Vec<usize>,
    pub density: Vec<usize>,
    pub stability: Vec<usize>,
    pub with_image: usize,
}

#[cfg(test)]
pub mod testing {
    use serde_json::json;
    use zylotek_contracts::woods::WoodResponse;

    /// A wood with the figures browsing reads; `None` leaves a figure out.
    pub fn wood(
        name: &str,
        family: &str,
        color: &str,
        place: &str,
        janka: Option<f64>,
        density: Option<f64>,
        shrinkage: Option<f64>,
    ) -> WoodResponse {
        serde_json::from_value(json!({
            "id": format!("id-{}", name.to_lowercase()),
            "name": name, "scientific_name": format!("{name} scientificus"), "genus": name,
            "family": family, "collection_location": place, "common_names": [format!("{name}-alias")],
            "images": {"main": if name == "Ipê" { json!("https://img.example/ipe.jpg") } else { json!(null) }},
            "description": {
                "heartwood_color_classification": color,
                "end_uses": format!("Used for decks and {name} floors"),
                "general_characteristics": {"grain": "straight"},
            },
            "physical_properties": {"density": {"apparent": density}, "shrinkage": {"volumetric": shrinkage}},
            "mechanical_properties": {"dry": {"janka_hardness": {"parallel_to_grain": janka},
                "static_bending": {"modulus_of_rupture": 900.0, "modulus_of_elasticity": 90000.0}}},
        }))
        .unwrap()
    }

    /// Five woods that differ in every way the filters look at.
    pub fn catalogue() -> Vec<WoodResponse> {
        vec![
            wood(
                "Ipê",
                "Bignoniaceae",
                "Marrom",
                "Santarém, PA",
                Some(1800.0),
                Some(1.05),
                Some(12.0),
            ),
            wood(
                "Cedro",
                "Meliaceae",
                "rosa",
                "Curuá-Una, PA",
                Some(450.0),
                Some(0.45),
                Some(8.0),
            ),
            wood(
                "Açaí",
                "Arecaceae",
                "marrom",
                "Juruá, AM",
                Some(900.0),
                Some(0.7),
                Some(16.0),
            ),
            wood("Pinus", "Pinaceae", "amarela", "Brasília", None, None, None),
            wood(
                "Jatobá",
                "Fabaceae",
                "marrom",
                "Floresta, RO",
                Some(1200.0),
                Some(0.9),
                Some(10.0),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::*, *};

    fn names(results: &Results) -> Vec<String> {
        results.shown.iter().map(|e| e.wood.name.clone()).collect()
    }

    fn run(raw: &str) -> Vec<String> {
        let woods = catalogue();
        let entries: Vec<Entry> = woods.iter().map(Entry::new).collect();

        names(&Query::parse(Some(raw)).search(&entries))
    }

    #[test]
    fn classes_have_keys_and_bands() {
        for class in Hardness::ALL {
            assert_eq!(Hardness::from_key(class.key()), Some(*class));
            assert_eq!(Hardness::ALL[class.index()], *class);
        }
        assert_eq!(Hardness::from_key("nope"), None);
        assert_eq!(
            [100.0, 500.0, 999.0, 1000.0, 1499.0, 1500.0].map(Hardness::of),
            [
                Hardness::Soft,
                Hardness::Medium,
                Hardness::Medium,
                Hardness::Hard,
                Hardness::Hard,
                Hardness::VeryHard
            ]
        );
        assert_eq!(
            [0.3, 0.55, 0.79, 0.8].map(Density::of),
            [
                Density::Light,
                Density::Medium,
                Density::Medium,
                Density::Heavy
            ]
        );
        assert_eq!(
            [5.0, 10.0, 14.9, 15.0].map(Stability::of),
            [
                Stability::Stable,
                Stability::Moderate,
                Stability::Moderate,
                Stability::Unstable
            ]
        );
    }

    #[test]
    fn folding_drops_case_and_accents() {
        assert_eq!(
            fold("Ipê Açaí Pão Ñandú Côco Ürsula Bébé Íris Òleo"),
            "ipe acai pao nandu coco ursula bebe iris oleo"
        );
    }

    #[test]
    fn an_entry_works_out_its_figures_words_and_region() {
        let woods = catalogue();
        let ipe = Entry::new(&woods[0]);
        let pinus = Entry::new(&woods[3]);

        assert_eq!(ipe.hardness, Some(1800.0));
        assert_eq!(ipe.hardness_class(), Some(Hardness::VeryHard));
        assert_eq!(ipe.density_class(), Some(Density::Heavy));
        assert_eq!(ipe.stability_class(), Some(Stability::Moderate));
        assert_eq!(
            ipe.color.as_deref(),
            Some("marrom"),
            "colours are lowercase"
        );
        assert_eq!(ipe.region.as_deref(), Some("PA"));
        assert_eq!(ipe.family.as_deref(), Some("Bignoniaceae"));
        assert!(ipe.image.is_some() && pinus.image.is_none());
        assert_eq!(pinus.region, None, "a place without a state has no region");
        assert_eq!(
            (
                pinus.hardness_class(),
                pinus.density_class(),
                pinus.stability_class()
            ),
            (None, None, None)
        );
        assert_eq!(ipe.bending, Some(900.0));
        assert_eq!(ipe.stiffness, Some(90000.0));
    }

    #[test]
    fn green_figures_stand_in_for_missing_dry_ones_and_junk_is_dropped() {
        let mut wood = wood(
            "Teste",
            "",
            "  ",
            "Lugar, XYZ",
            None,
            Some(0.5),
            Some(f64::NAN),
        );
        wood.details
            .mechanical_properties
            .green
            .janka_hardness
            .parallel_to_grain = Some(700.0);

        let entry = Entry::new(&wood);

        assert_eq!(entry.hardness, Some(700.0));
        assert_eq!(entry.shrinkage, None, "NaN is not a figure");
        assert_eq!(
            (entry.family, entry.color, entry.region),
            (None, None, None)
        );
    }

    #[test]
    fn a_query_is_read_from_the_url_and_junk_is_ignored() {
        let query = Query::parse(Some(
            "q=%20ipe%20&family=Meliaceae&family=Meliaceae&color=Marrom&region=pa&hardness=hard&hardness=bogus\
             &density=heavy&stability=stable&hardness_min=500&hardness_max=-3&density_min=x&density_max=1.2\
             &image=1&sort=hardest&view=list&page=3&unknown=1&family=",
        ));

        assert_eq!(query.text, "ipe");
        assert_eq!(query.families, ["Meliaceae"], "no repeats, no blanks");
        assert_eq!(query.colors, ["marrom"]);
        assert_eq!(query.regions, ["PA"]);
        assert_eq!(query.hardness, [Hardness::Hard]);
        assert_eq!(query.density, [Density::Heavy]);
        assert_eq!(query.stability, [Stability::Stable]);
        assert_eq!(
            (query.hardness_min, query.hardness_max),
            (Some(500.0), None)
        );
        assert_eq!((query.density_min, query.density_max), (None, Some(1.2)));
        assert!(query.with_image);
        assert_eq!(
            (query.sort(), query.view(), query.page),
            (Sort::Hardest, View::List, 3)
        );
        let default = Query::parse(Some("page=0&sort=bogus&view=bogus&image=2"));
        assert_eq!(
            (
                default.sort(),
                default.view(),
                default.page,
                default.with_image
            ),
            (Sort::Name, View::Cards, 1, false)
        );
        assert_eq!(
            Query::parse(Some("q=a")).sort(),
            Sort::Relevance,
            "searching sorts by relevance"
        );
        assert_eq!(
            Query::parse(None),
            Query {
                page: 1,
                ..Query::default()
            }
        );
    }

    #[test]
    fn a_query_is_bounded() {
        let many: String = (0..50).map(|n| format!("family=F{n}&")).collect();
        let long = format!("q={}", "x".repeat(1000));

        assert_eq!(Query::parse(Some(&many)).families.len(), MAX_VALUES);
        assert_eq!(
            Query::parse(Some(&long)).text.chars().count(),
            MAX_TEXT_CHARS
        );
    }

    #[test]
    fn a_query_becomes_an_address_and_back() {
        assert_eq!(Query::parse(None).url(), "/woods");
        let raw = "q=a+b&family=Meliaceae&color=marrom&region=PA&hardness=hard&density=light&stability=stable\
                   &hardness_min=500&hardness_max=900.5&density_min=0.4&density_max=0.9&image=1&sort=hardest&view=list&page=2";
        let query = Query::parse(Some(raw));

        let url = query.url();

        assert_eq!(url, format!("/woods?{raw}"));
        assert_eq!(Query::parse(url.strip_prefix("/woods?")), query);
        assert_eq!(
            Query::parse(Some("view=cards")).url(),
            "/woods",
            "defaults are left out"
        );
        assert!(Query::parse(Some("q=%C3%AAx")).url().contains("q=%C3%AAx"));
    }

    #[test]
    fn a_link_changes_one_thing_and_goes_back_to_the_first_page() {
        let query = Query::parse(Some("q=ipe&page=3&view=list"));

        assert_eq!(
            query.link(|q| q.hardness.push(Hardness::Soft)),
            "/woods?q=ipe&hardness=soft&view=list"
        );
        assert_eq!(query.link(|q| q.page = 4), "/woods?q=ipe&view=list&page=4");
        assert_eq!(query.link(|q| q.text.clear()), "/woods?view=list");
    }

    #[test]
    fn unfiltered_means_nothing_narrows_the_list() {
        assert!(Query::parse(None).is_unfiltered());
        assert!(Query::parse(Some("view=list&sort=hardest&page=2")).is_unfiltered());
        for narrowed in [
            "q=a",
            "family=A",
            "hardness=soft",
            "hardness_min=1",
            "image=1",
        ] {
            assert!(!Query::parse(Some(narrowed)).is_unfiltered(), "{narrowed}");
        }
    }

    #[test]
    fn text_search_is_accent_insensitive_and_needs_every_word() {
        assert_eq!(run("q=ipe"), ["Ipê"]);
        assert_eq!(run("q=ACAI"), ["Açaí"]);
        assert_eq!(run("q=meliaceae"), ["Cedro"], "families are searched");
        assert_eq!(run("q=santarem"), ["Ipê"], "so are places");
        assert_eq!(run("q=marrom"), ["Açaí", "Ipê", "Jatobá"], "and colours");
        assert_eq!(
            run("q=decks+jatoba"),
            ["Jatobá"],
            "end uses count, and every word must match"
        );
        assert_eq!(
            run("q=straight"),
            ["Açaí", "Cedro", "Ipê", "Jatobá", "Pinus"],
            "and characteristics"
        );
        assert_eq!(run("q=cedro+ipe"), Vec::<String>::new());
        assert_eq!(run("q=ipe+alias"), ["Ipê"], "common names");
    }

    #[test]
    fn filters_narrow_and_combine() {
        assert_eq!(
            run("family=Meliaceae&family=Pinaceae"),
            ["Cedro", "Pinus"],
            "values of one filter are alternatives"
        );
        assert_eq!(
            run("color=marrom&region=PA"),
            ["Ipê"],
            "different filters all apply"
        );
        assert_eq!(run("hardness=very-hard"), ["Ipê"]);
        assert_eq!(run("hardness=soft&hardness=hard"), ["Cedro", "Jatobá"]);
        assert_eq!(run("density=light"), ["Cedro"]);
        assert_eq!(run("stability=unstable"), ["Açaí"]);
        assert_eq!(
            run("hardness_min=800&hardness_max=1300"),
            ["Açaí", "Jatobá"]
        );
        assert_eq!(run("density_max=0.5"), ["Cedro"]);
        assert_eq!(
            run("hardness_min=0"),
            ["Açaí", "Cedro", "Ipê", "Jatobá"],
            "a range drops woods without the figure"
        );
        assert_eq!(run("image=1"), ["Ipê"]);
        assert_eq!(run("family=Nothing"), Vec::<String>::new());
    }

    #[test]
    fn sorting_puts_woods_without_the_figure_last() {
        assert_eq!(
            run(""),
            ["Açaí", "Cedro", "Ipê", "Jatobá", "Pinus"],
            "by name, ignoring accents"
        );
        assert_eq!(
            run("sort=name-desc"),
            ["Pinus", "Jatobá", "Ipê", "Cedro", "Açaí"]
        );
        assert_eq!(
            run("sort=hardest"),
            ["Ipê", "Jatobá", "Açaí", "Cedro", "Pinus"]
        );
        assert_eq!(
            run("sort=softest"),
            ["Cedro", "Açaí", "Jatobá", "Ipê", "Pinus"]
        );
        assert_eq!(
            run("sort=heaviest"),
            ["Ipê", "Jatobá", "Açaí", "Cedro", "Pinus"]
        );
        assert_eq!(
            run("sort=lightest"),
            ["Cedro", "Açaí", "Jatobá", "Ipê", "Pinus"]
        );
        assert_eq!(
            run("sort=stablest"),
            ["Cedro", "Jatobá", "Ipê", "Açaí", "Pinus"]
        );
    }

    #[test]
    fn woods_without_the_figure_stay_together_at_the_end_in_name_order() {
        let mut woods = catalogue();
        woods.push(wood("Abiu", "F", "c", "X, PA", None, None, None));
        let entries: Vec<Entry> = woods.iter().map(Entry::new).collect();

        let found = names(&Query::parse(Some("sort=hardest")).search(&entries));

        assert_eq!(found, ["Ipê", "Jatobá", "Açaí", "Cedro", "Abiu", "Pinus"]);
    }

    #[test]
    fn relevance_puts_names_before_mentions() {
        // "ipe" is Ipê's name; the others only mention it in passing.
        let mut woods = catalogue();
        woods[1].details.description.end_uses = Some("Looks like ipe".into());
        woods[4].name = "Falso-ipê".into();
        let entries: Vec<Entry> = woods.iter().map(Entry::new).collect();

        let found = names(&Query::parse(Some("q=ipe")).search(&entries));

        assert_eq!(found, ["Ipê", "Falso-ipê", "Cedro"]);
    }

    #[test]
    fn a_scientific_name_ranks_between_a_name_and_a_mention() {
        let mut woods = catalogue();
        woods[2].scientific_name = "Cedrela fissilis".into();
        woods[4].details.description.end_uses = Some("Resembles cedro".into());
        let entries: Vec<Entry> = woods.iter().map(Entry::new).collect();

        let found = names(&Query::parse(Some("q=cedr")).search(&entries));

        assert_eq!(found, ["Cedro", "Açaí", "Jatobá"]);
    }

    #[test]
    fn results_are_paged_and_a_page_past_the_end_shows_the_last() {
        let woods: Vec<_> = (0..60)
            .map(|n| {
                wood(
                    &format!("W{n:02}"),
                    "F",
                    "c",
                    "X, PA",
                    Some(100.0),
                    Some(0.5),
                    Some(5.0),
                )
            })
            .collect();
        let entries: Vec<Entry> = woods.iter().map(Entry::new).collect();

        let first = Query::parse(None).search(&entries);
        let last = Query::parse(Some("page=3")).search(&entries);
        let past = Query::parse(Some("page=99")).search(&entries);
        let none = Query::parse(Some("q=zzz")).search(&entries);

        assert_eq!(
            (first.found, first.pages, first.page, first.shown.len()),
            (60, 3, 1, PAGE_SIZE)
        );
        assert_eq!(
            (last.page, last.shown.len(), names(&last)[0].as_str()),
            (3, 12, "W48")
        );
        assert_eq!((past.page, past.shown.len()), (3, 12));
        assert_eq!(
            (none.found, none.pages, none.page, none.shown.len()),
            (0, 1, 1, 0)
        );
    }

    #[test]
    fn facet_counts_ignore_their_own_choice_but_respect_the_others() {
        let woods = catalogue();
        let entries: Vec<Entry> = woods.iter().map(Entry::new).collect();

        let all = Query::parse(None).search(&entries).facets;
        let narrowed = Query::parse(Some("color=marrom&family=Pinaceae"))
            .search(&entries)
            .facets;

        assert_eq!(all.families.len(), 5);
        assert_eq!(
            all.colors,
            [
                ("marrom".to_owned(), 3),
                ("amarela".to_owned(), 1),
                ("rosa".to_owned(), 1)
            ]
        );
        assert_eq!(
            all.regions,
            [
                ("PA".to_owned(), 2),
                ("AM".to_owned(), 1),
                ("RO".to_owned(), 1)
            ]
        );
        assert_eq!(all.hardness, [1, 1, 1, 1]);
        assert_eq!(all.density, [1, 1, 2]);
        assert_eq!(all.stability, [1, 2, 1]);
        assert_eq!(all.with_image, 1);
        // Colours are counted as if only the family were chosen, and the other way round.
        assert_eq!(
            narrowed.colors,
            [("amarela".to_owned(), 1), ("marrom".to_owned(), 0)]
        );
        assert_eq!(narrowed.families.iter().map(|f| f.1).sum::<usize>(), 3);
        assert!(
            narrowed.families.contains(&("Pinaceae".to_owned(), 0)),
            "a chosen value stays on the list"
        );
    }
}
