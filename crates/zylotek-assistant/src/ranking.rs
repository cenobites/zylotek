//! Ranks the catalogue for a set of weighted criteria. Plain arithmetic over the figures in
//! the database: every property is turned into a percentile among all woods that have the
//! figure, so units never need comparing and the same question always gives the same list.

use std::collections::BTreeMap;

use zylotek_contracts::{
    assistant::{Property, Reason, Recommendation},
    woods::{WoodResponse, WorkabilityRating},
};

use crate::requirements::Requirements;

pub struct Ranker {
    woods: Vec<WoodResponse>,
    /// Every known figure of each property, ascending.
    figures: BTreeMap<Property, Vec<f64>>,
}

impl Ranker {
    pub fn new(woods: Vec<WoodResponse>) -> Self {
        let figures = Property::ALL
            .into_iter()
            .map(|property| {
                let mut values: Vec<f64> = woods
                    .iter()
                    .filter_map(|wood| figure(property, wood))
                    .collect();
                values.sort_by(f64::total_cmp);

                (property, values)
            })
            .collect();

        Self { woods, figures }
    }

    pub fn len(&self) -> usize {
        self.woods.len()
    }

    pub fn is_empty(&self) -> bool {
        self.woods.is_empty()
    }

    /// The best `limit` woods, best first. A wood with no figure for any criterion is left
    /// out; one with figures for only some is held back by the zeros for the rest. Ties
    /// break on name, then id, so the order never wobbles.
    pub fn rank(&self, requirements: &Requirements, limit: usize) -> Vec<Recommendation> {
        let total_weight: f64 = requirements
            .criteria
            .iter()
            .map(|criterion| f64::from(criterion.weight))
            .sum();

        let mut ranked: Vec<Recommendation> = self
            .woods
            .iter()
            .filter(|wood| !is_avoided(wood, &requirements.avoid))
            .filter_map(|wood| {
                let mut reasons = Vec::new();
                let mut earned = 0.0;
                for criterion in &requirements.criteria {
                    let Some(value) = figure(criterion.property, wood) else {
                        continue;
                    };
                    let fit = self.fit(criterion.property, value);
                    earned += f64::from(criterion.weight) * fit;
                    reasons.push(Reason {
                        property: criterion.property,
                        value,
                        fit,
                    });
                }

                (!reasons.is_empty()).then(|| Recommendation {
                    wood_id: wood.id.clone(),
                    name: wood.name.clone(),
                    scientific_name: wood.scientific_name.clone(),
                    score: earned / total_weight,
                    reasons,
                })
            })
            .collect();

        ranked.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.wood_id.cmp(&b.wood_id))
        });
        ranked.truncate(limit);

        ranked
    }

    /// 0 worst to 1 best: the share of known figures below this one, half of those equal to
    /// it, flipped for properties where less is better.
    fn fit(&self, property: Property, value: f64) -> f64 {
        let known = &self.figures[&property];
        let below = known.partition_point(|figure| *figure < value);
        let up_to = known.partition_point(|figure| *figure <= value);
        let percentile = (below as f64 + (up_to - below) as f64 / 2.0) / known.len() as f64;

        if lower_is_better(property) {
            1.0 - percentile
        } else {
            percentile
        }
    }
}

fn lower_is_better(property: Property) -> bool {
    matches!(property, Property::Stability | Property::Lightness)
}

fn is_avoided(wood: &WoodResponse, avoid: &[String]) -> bool {
    let names = std::iter::once(&wood.name)
        .chain(std::iter::once(&wood.scientific_name))
        .chain(&wood.details.common_names)
        .map(|name| name.to_lowercase())
        .collect::<Vec<_>>();

    avoid
        .iter()
        .any(|avoided| names.iter().any(|name| name.contains(avoided)))
}

/// The catalogue's figure for a property, if it has one. Dry-wood figures are preferred
/// because that is how furniture wood is used; green is the fallback.
pub fn figure(property: Property, wood: &WoodResponse) -> Option<f64> {
    let mechanical = &wood.details.mechanical_properties;
    let value = match property {
        Property::Hardness => mechanical
            .dry
            .janka_hardness
            .parallel_to_grain
            .or(mechanical.green.janka_hardness.parallel_to_grain),
        Property::Strength => mechanical
            .dry
            .static_bending
            .modulus_of_rupture
            .or(mechanical.green.static_bending.modulus_of_rupture),
        Property::Stiffness => mechanical
            .dry
            .static_bending
            .modulus_of_elasticity
            .or(mechanical.green.static_bending.modulus_of_elasticity),
        Property::Stability => wood.details.physical_properties.shrinkage.volumetric,
        Property::Lightness => {
            let density = &wood.details.physical_properties.density;
            density.apparent.or(density.dry).or(density.basic)
        }
        Property::Workability => workability(wood),
    };

    value.filter(|value| value.is_finite())
}

/// A figure as the explanation prompt shows it: what it measures and which way is better, so
/// the model does not read a density as a "lightness score" or a shrinkage as a quality.
pub fn describe(property: Property, value: f64) -> String {
    match property {
        Property::Hardness => format!("hardness (Janka) {value:.0}, higher is harder"),
        Property::Strength => format!("bending strength {value:.0}, higher is stronger"),
        Property::Stiffness => {
            format!("stiffness (modulus of elasticity) {value:.0}, higher is stiffer")
        }
        Property::Stability => {
            format!("dimensional stability, volumetric shrinkage {value:.1}%, lower is more stable")
        }
        Property::Lightness => format!("lightness, density {value:.2} g/cm³, lower is lighter"),
        Property::Workability => format!("workability {value:.2} out of 1, higher is easier"),
    }
}

/// The mean of the hand-work ratings (planing, sanding, clamping, drilling), 0 to 1.
fn workability(wood: &WoodResponse) -> Option<f64> {
    let surface = &wood.details.workabilities.surface_finishing;
    let ratings: Vec<f64> = [
        surface.planer,
        surface.sandpaper,
        surface.vise,
        surface.drill,
    ]
    .into_iter()
    .filter_map(rating)
    .collect();

    (!ratings.is_empty()).then(|| ratings.iter().sum::<f64>() / ratings.len() as f64)
}

fn rating(rating: WorkabilityRating) -> Option<f64> {
    let step = match rating {
        WorkabilityRating::NotApplicable => return None,
        WorkabilityRating::VeryPoor => 0,
        WorkabilityRating::Poor => 1,
        WorkabilityRating::Fair => 2,
        WorkabilityRating::Easy => 3,
        WorkabilityRating::Good => 4,
        WorkabilityRating::VeryGood => 5,
        WorkabilityRating::Excellent => 6,
    };

    Some(f64::from(step) / 6.0)
}

#[cfg(test)]
pub mod testing {
    use zylotek_contracts::woods::WoodDetailsSchema;

    use super::*;

    pub fn wood(name: &str) -> WoodResponse {
        WoodResponse {
            id: format!("id-{name}"),
            name: name.to_owned(),
            scientific_name: format!("{name} scientificus"),
            genus: name.to_owned(),
            details: WoodDetailsSchema::default(),
        }
    }

    pub fn hardness(name: &str, janka: f64) -> WoodResponse {
        let mut wood = wood(name);
        wood.details
            .mechanical_properties
            .dry
            .janka_hardness
            .parallel_to_grain = Some(janka);

        wood
    }
}

#[cfg(test)]
mod tests {
    use zylotek_contracts::assistant::Criterion;

    use super::{testing::*, *};

    fn want(criteria: &[(Property, u8)]) -> Requirements {
        Requirements {
            summary: String::new(),
            criteria: criteria
                .iter()
                .map(|&(property, weight)| Criterion { property, weight })
                .collect(),
            avoid: Vec::new(),
        }
    }

    fn names(ranked: &[Recommendation]) -> Vec<&str> {
        ranked.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn every_property_is_described_with_what_it_measures_and_which_way_is_better() {
        let described: Vec<String> = Property::ALL
            .into_iter()
            .map(|property| describe(property, 1234.5678))
            .collect();

        assert_eq!(
            described,
            [
                "hardness (Janka) 1235, higher is harder",
                "bending strength 1235, higher is stronger",
                "stiffness (modulus of elasticity) 1235, higher is stiffer",
                "dimensional stability, volumetric shrinkage 1234.6%, lower is more stable",
                "lightness, density 1234.57 g/cm³, lower is lighter",
                "workability 1234.57 out of 1, higher is easier",
            ]
        );
    }

    #[test]
    fn harder_wood_ranks_first_when_hardness_is_wanted() {
        let ranker = Ranker::new(vec![
            hardness("Soft", 300.0),
            hardness("Hard", 1500.0),
            hardness("Middle", 900.0),
        ]);

        let ranked = ranker.rank(&want(&[(Property::Hardness, 3)]), 10);

        assert_eq!(names(&ranked), ["Hard", "Middle", "Soft"]);
        assert_eq!(ranked[0].reasons[0].value, 1500.0);
        assert!(ranked[0].score > ranked[1].score && ranked[1].score > ranked[2].score);
        assert_eq!(ranker.len(), 3);
        assert!(!ranker.is_empty());
    }

    #[test]
    fn fit_is_the_percentile_with_ties_sharing_half() {
        let ranker = Ranker::new(vec![
            hardness("A", 100.0),
            hardness("B", 200.0),
            hardness("C", 200.0),
            hardness("D", 400.0),
        ]);

        assert_eq!(ranker.fit(Property::Hardness, 100.0), 0.125);
        assert_eq!(ranker.fit(Property::Hardness, 200.0), 0.5);
        assert_eq!(ranker.fit(Property::Hardness, 400.0), 0.875);
    }

    #[test]
    fn for_stability_and_lightness_less_is_better() {
        let mut calm = wood("Calm");
        calm.details.physical_properties.shrinkage.volumetric = Some(6.0);
        calm.details.physical_properties.density.apparent = Some(0.4);
        let mut restless = wood("Restless");
        restless.details.physical_properties.shrinkage.volumetric = Some(18.0);
        restless.details.physical_properties.density.apparent = Some(1.1);
        let ranker = Ranker::new(vec![restless, calm]);

        let stable = ranker.rank(&want(&[(Property::Stability, 2)]), 10);
        let light = ranker.rank(&want(&[(Property::Lightness, 2)]), 10);

        assert_eq!(names(&stable), ["Calm", "Restless"]);
        assert_eq!(names(&light), ["Calm", "Restless"]);
        assert!(stable[0].reasons[0].fit > 0.5 && stable[1].reasons[0].fit < 0.5);
    }

    #[test]
    fn weights_decide_between_competing_properties() {
        let mut hard = hardness("Hard", 2000.0);
        hard.details.physical_properties.shrinkage.volumetric = Some(20.0);
        let mut steady = hardness("Steady", 500.0);
        steady.details.physical_properties.shrinkage.volumetric = Some(5.0);
        let ranker = Ranker::new(vec![hard, steady]);

        let wants_hard = ranker.rank(
            &want(&[(Property::Hardness, 3), (Property::Stability, 1)]),
            2,
        );
        let wants_steady = ranker.rank(
            &want(&[(Property::Hardness, 1), (Property::Stability, 3)]),
            2,
        );

        assert_eq!(names(&wants_hard), ["Hard", "Steady"]);
        assert_eq!(names(&wants_steady), ["Steady", "Hard"]);
    }

    #[test]
    fn a_missing_figure_counts_as_zero_and_no_figures_means_left_out() {
        let mut full = hardness("Full", 1000.0);
        full.details.physical_properties.shrinkage.volumetric = Some(10.0);
        let partial = hardness("Partial", 1000.0);
        let none = wood("None");
        let ranker = Ranker::new(vec![none, partial, full]);

        let ranked = ranker.rank(
            &want(&[(Property::Hardness, 1), (Property::Stability, 1)]),
            10,
        );

        assert_eq!(names(&ranked), ["Full", "Partial"]);
        assert_eq!(ranked[1].reasons.len(), 1);
        assert!(ranked[0].score > ranked[1].score);
    }

    #[test]
    fn avoided_names_are_matched_on_any_name_the_wood_has() {
        let mut ipe = hardness("Ipê", 3000.0);
        ipe.details.common_names = vec!["Lapacho".into()];
        let ranker = Ranker::new(vec![
            ipe,
            hardness("Pinus", 400.0),
            hardness("Cedro", 500.0),
        ]);
        let mut requirements = want(&[(Property::Hardness, 3)]);

        requirements.avoid = vec!["lapacho".into()];
        assert_eq!(names(&ranker.rank(&requirements, 10)), ["Cedro", "Pinus"]);
        requirements.avoid = vec!["pinus scientificus".into()];
        assert_eq!(names(&ranker.rank(&requirements, 10)), ["Ipê", "Cedro"]);
        requirements.avoid = vec!["ipê".into()];
        assert_eq!(names(&ranker.rank(&requirements, 10)), ["Cedro", "Pinus"]);
    }

    #[test]
    fn the_limit_truncates_and_ties_break_on_name_then_id() {
        let mut twin = hardness("Alpha", 100.0);
        twin.id = "z".into();
        let mut other = hardness("Alpha", 100.0);
        other.id = "a".into();
        let ranker = Ranker::new(vec![hardness("Beta", 100.0), twin, other]);

        let ranked = ranker.rank(&want(&[(Property::Hardness, 1)]), 2);

        assert_eq!(
            ranked
                .iter()
                .map(|r| r.wood_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
        assert!(ranker.rank(&want(&[(Property::Hardness, 1)]), 0).is_empty());
    }

    #[test]
    fn an_empty_catalogue_ranks_nothing() {
        let ranker = Ranker::new(Vec::new());

        assert!(ranker.is_empty());
        assert!(ranker.rank(&want(&[(Property::Hardness, 1)]), 5).is_empty());
    }

    #[test]
    fn green_figures_stand_in_when_dry_ones_are_missing() {
        let mut green_only = wood("Green");
        green_only
            .details
            .mechanical_properties
            .green
            .janka_hardness
            .parallel_to_grain = Some(700.0);
        green_only
            .details
            .mechanical_properties
            .green
            .static_bending
            .modulus_of_rupture = Some(800.0);
        green_only
            .details
            .mechanical_properties
            .green
            .static_bending
            .modulus_of_elasticity = Some(90000.0);
        let mut dry = green_only.clone();
        dry.details
            .mechanical_properties
            .dry
            .janka_hardness
            .parallel_to_grain = Some(900.0);
        dry.details
            .mechanical_properties
            .dry
            .static_bending
            .modulus_of_rupture = Some(1000.0);
        dry.details
            .mechanical_properties
            .dry
            .static_bending
            .modulus_of_elasticity = Some(100000.0);

        assert_eq!(figure(Property::Hardness, &green_only), Some(700.0));
        assert_eq!(figure(Property::Strength, &green_only), Some(800.0));
        assert_eq!(figure(Property::Stiffness, &green_only), Some(90000.0));
        assert_eq!(figure(Property::Hardness, &dry), Some(900.0));
        assert_eq!(figure(Property::Strength, &dry), Some(1000.0));
        assert_eq!(figure(Property::Stiffness, &dry), Some(100000.0));
    }

    #[test]
    fn density_prefers_apparent_then_dry_then_basic() {
        let mut w = wood("D");
        assert_eq!(figure(Property::Lightness, &w), None);
        w.details.physical_properties.density.basic = Some(0.5);
        assert_eq!(figure(Property::Lightness, &w), Some(0.5));
        w.details.physical_properties.density.dry = Some(0.6);
        assert_eq!(figure(Property::Lightness, &w), Some(0.6));
        w.details.physical_properties.density.apparent = Some(0.7);
        assert_eq!(figure(Property::Lightness, &w), Some(0.7));
    }

    #[test]
    fn workability_averages_the_hand_work_ratings_and_skips_unrated_ones() {
        let mut w = wood("W");
        assert_eq!(figure(Property::Workability, &w), None);
        w.details.workabilities.surface_finishing.planer = WorkabilityRating::Excellent;
        assert_eq!(figure(Property::Workability, &w), Some(1.0));
        w.details.workabilities.surface_finishing.sandpaper = WorkabilityRating::VeryPoor;
        assert_eq!(figure(Property::Workability, &w), Some(0.5));

        for (given, expected) in [
            (WorkabilityRating::Poor, 1.0 / 6.0),
            (WorkabilityRating::Fair, 2.0 / 6.0),
            (WorkabilityRating::Easy, 0.5),
            (WorkabilityRating::Good, 4.0 / 6.0),
            (WorkabilityRating::VeryGood, 5.0 / 6.0),
        ] {
            assert_eq!(rating(given), Some(expected));
        }
        assert_eq!(rating(WorkabilityRating::NotApplicable), None);
    }

    #[test]
    fn a_non_finite_figure_is_ignored() {
        let mut w = hardness("Nan", f64::NAN);
        assert_eq!(figure(Property::Hardness, &w), None);
        w.details
            .mechanical_properties
            .dry
            .janka_hardness
            .parallel_to_grain = Some(f64::INFINITY);
        assert_eq!(figure(Property::Hardness, &w), None);
    }
}
