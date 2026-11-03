use crate::woods::value_objects::{
    Attributes, CommonNames, KilnDrying, MechanicalProperties, MechanicalPropertiesByCondition,
    PhysicalProperties, WoodDescription, WoodDetails, WoodImages, Workabilities,
};

impl WoodDetails {
    /// These details with every missing value taken from `other`.
    ///
    /// A value that is present here is never replaced, so `self` wins wherever both have
    /// one. Labelled attributes and common names are pooled, with `self`'s taking
    /// precedence on a clash. This is how a second source completes the first, and how a
    /// re-import keeps what an earlier import stored but the new data lacks.
    pub fn fill_gaps_from(self, other: WoodDetails) -> WoodDetails {
        fill_details(self, other)
    }
}

/// `slot` keeps its value; `other` only supplies a missing one.
fn fill<T>(slot: &mut Option<T>, other: Option<T>) {
    if slot.is_none() {
        *slot = other;
    }
}

/// Like [`fill`], for values whose "missing" is their default (unrated, empty).
fn fill_default<T: Default + PartialEq>(slot: &mut T, other: T) {
    if *slot == T::default() {
        *slot = other;
    }
}

fn fill_details(mut d: WoodDetails, other: WoodDetails) -> WoodDetails {
    fill(&mut d.family, other.family);
    fill(&mut d.collection_location, other.collection_location);
    d.common_names = CommonNames::new(
        d.common_names
            .as_slice()
            .iter()
            .chain(other.common_names.as_slice()),
    );
    // The species page is a better source than the dataset's landing page, and its URL and
    // id belong together.
    if other.source.is_some() {
        d.source = other.source;
    }
    d.description = fill_description(d.description, other.description);
    d.kiln_drying = KilnDrying {
        schedule: union(&d.kiln_drying.schedule, &other.kiln_drying.schedule),
        duration: d.kiln_drying.duration.or(other.kiln_drying.duration),
    };
    d.images = fill_images(d.images, other.images);
    d.physical_properties = fill_physical(d.physical_properties, other.physical_properties);
    d.mechanical_properties = MechanicalProperties {
        green: fill_by_condition(
            d.mechanical_properties.green,
            other.mechanical_properties.green,
        ),
        dry: fill_by_condition(d.mechanical_properties.dry, other.mechanical_properties.dry),
    };
    d.workabilities = fill_workabilities(d.workabilities, other.workabilities);

    d
}

/// All of `primary`'s attributes, plus the ones only `other` has.
fn union(primary: &Attributes, other: &Attributes) -> Attributes {
    // Later pairs replace earlier ones, so the primary comes last.
    Attributes::new(other.iter().chain(primary.iter()))
}

fn fill_description(mut d: WoodDescription, other: WoodDescription) -> WoodDescription {
    d.tree = union(&d.tree, &other.tree);
    d.general_characteristics = union(&d.general_characteristics, &other.general_characteristics);
    fill(&mut d.end_uses, other.end_uses);
    fill(&mut d.preservation, other.preservation);
    fill(&mut d.workability_notes, other.workability_notes);
    fill(
        &mut d.heartwood_color_classification,
        other.heartwood_color_classification,
    );

    d
}

fn fill_images(mut d: WoodImages, other: WoodImages) -> WoodImages {
    fill(&mut d.main, other.main);
    fill(&mut d.tree, other.tree);
    fill(&mut d.log, other.log);
    fill(&mut d.bark, other.bark);
    fill(&mut d.tangential_cut, other.tangential_cut);
    fill(&mut d.cross_section, other.cross_section);
    fill(&mut d.radial_cut, other.radial_cut);
    fill(&mut d.wood, other.wood);

    d
}

fn fill_physical(mut d: PhysicalProperties, other: PhysicalProperties) -> PhysicalProperties {
    fill(&mut d.density.dry, other.density.dry);
    fill(&mut d.density.green, other.density.green);
    fill(&mut d.density.basic, other.density.basic);
    fill(&mut d.density.apparent, other.density.apparent);
    fill(&mut d.shrinkage.tangential, other.shrinkage.tangential);
    fill(&mut d.shrinkage.radial, other.shrinkage.radial);
    fill(&mut d.shrinkage.volumetric, other.shrinkage.volumetric);

    d
}

fn fill_by_condition(
    mut d: MechanicalPropertiesByCondition,
    other: MechanicalPropertiesByCondition,
) -> MechanicalPropertiesByCondition {
    fill(
        &mut d.static_bending.modulus_of_rupture,
        other.static_bending.modulus_of_rupture,
    );
    fill(
        &mut d.static_bending.modulus_of_elasticity,
        other.static_bending.modulus_of_elasticity,
    );
    fill(
        &mut d.compression.parallel_to_grain,
        other.compression.parallel_to_grain,
    );
    fill(
        &mut d.compression.perpendicular_to_grain,
        other.compression.perpendicular_to_grain,
    );
    fill(
        &mut d.janka_hardness.parallel_to_grain,
        other.janka_hardness.parallel_to_grain,
    );
    fill(
        &mut d.janka_hardness.end_grain,
        other.janka_hardness.end_grain,
    );
    fill(
        &mut d.tension_perpendicular_to_grain,
        other.tension_perpendicular_to_grain,
    );
    fill(&mut d.cleavage_strength, other.cleavage_strength);
    fill(
        &mut d.shear_parallel_to_grain,
        other.shear_parallel_to_grain,
    );
    fill(
        &mut d.nail_extraction.parallel_to_grain,
        other.nail_extraction.parallel_to_grain,
    );
    fill(
        &mut d.nail_extraction.end_grain,
        other.nail_extraction.end_grain,
    );

    d
}

fn fill_workabilities(mut d: Workabilities, other: Workabilities) -> Workabilities {
    let (e, w) = (&mut d.equipment_finishing, other.equipment_finishing);
    fill_default(&mut e.molding_planer, w.molding_planer);
    fill_default(&mut e.narrow_belt_sander, w.narrow_belt_sander);
    fill_default(&mut e.lathe, w.lathe);
    fill_default(&mut e.drill_bit, w.drill_bit);

    let (s, w) = (&mut d.surface_finishing, other.surface_finishing);
    fill_default(&mut s.planer, w.planer);
    fill_default(&mut s.sandpaper, w.sandpaper);
    fill_default(&mut s.vise, w.vise);
    fill_default(&mut s.drill, w.drill);
    fill_default(&mut s.nail_crack, w.nail_crack);

    let (t, w) = (&mut d.treatabilities, other.treatabilities);
    fill_default(&mut t.heartwood, w.heartwood);
    fill_default(&mut t.sapwood, w.sapwood);

    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::full_details;

    #[test]
    fn gaps_are_filled_from_the_other_details() {
        assert_eq!(
            WoodDetails::default().fill_gaps_from(full_details()),
            full_details()
        );
    }

    #[test]
    fn nothing_changes_when_there_is_nothing_to_fill() {
        assert_eq!(
            full_details().fill_gaps_from(WoodDetails::default()),
            full_details()
        );
        assert_eq!(
            WoodDetails::default().fill_gaps_from(WoodDetails::default()),
            WoodDetails::default()
        );
    }

    #[test]
    fn present_values_win_and_labelled_ones_are_pooled() {
        let mut mine = WoodDetails::default();
        mine.description.end_uses = Some("mine".to_owned());
        mine.description.tree = Attributes::new([("dap", "mine")]);
        mine.common_names = CommonNames::new(["Mine"]);
        let mut theirs = full_details();
        theirs.description.end_uses = Some("theirs".to_owned());
        theirs.description.tree = Attributes::new([("dap", "theirs"), ("altura", "theirs")]);

        let merged = mine.fill_gaps_from(theirs.clone());

        assert_eq!(merged.description.end_uses.as_deref(), Some("mine"));
        assert_eq!(merged.description.tree.get("dap"), Some("mine"));
        assert_eq!(merged.description.tree.get("altura"), Some("theirs"));
        assert_eq!(merged.common_names.as_slice()[0], "Mine");
        for name in theirs.common_names.as_slice() {
            assert!(merged.common_names.as_slice().contains(name));
        }
        assert_eq!(merged.images, theirs.images);
    }
}
