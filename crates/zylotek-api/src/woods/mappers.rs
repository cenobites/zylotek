//! Conversions between the domain and the wire shapes in `zylotek-contracts`.
//!
//! These are local traits rather than `From` impls because the orphan rule forbids
//! `From` between two types this crate does not own.

use std::collections::BTreeMap;

use zylotek_application::woods::list_woods::WoodPage;
use zylotek_contracts::woods::{
    DensitySchema, DescriptionSchema, EquipmentFinishingResultSchema, EquipmentFinishingSchema,
    GrainDirectionsSchema, GrainForceSchema, ImagesSchema, KilnDryingSchema,
    MechanicalPropertiesByConditionSchema, MechanicalPropertiesSchema, NailCrackBehavior,
    PhysicalPropertiesSchema, ShrinkageSchema, SourceSchema, StaticBendingSchema,
    SurfaceFinishingSchema, TreatabilitiesSchema, TreatabilityRating, WoodDetailsSchema,
    WoodPageResponse, WoodResponse, WorkabilitiesSchema, WorkabilityDefect,
    WorkabilityEquipmentSettings, WorkabilityRating,
};
use zylotek_domain::woods::{
    entities::Wood,
    errors::WoodError,
    value_objects::{self as vo, Measurement},
};

/// Domain value to wire shape, or wire shape to domain value when it cannot fail.
pub trait Convert<T> {
    fn convert(self) -> T;
}

/// Wire shape to domain value, validating on the way in.
pub trait TryConvert<T> {
    fn try_convert(self) -> Result<T, WoodError>;
}

/// Converts a domain enum to its wire twin and back, variant by variant.
macro_rules! mirror_enum {
    ($name:ident { $($variant:ident),* $(,)? }) => {
        impl Convert<$name> for vo::$name {
            fn convert(self) -> $name {
                match self {
                    vo::$name::NotApplicable => $name::NotApplicable,
                    $(vo::$name::$variant => $name::$variant,)*
                }
            }
        }

        impl Convert<vo::$name> for $name {
            fn convert(self) -> vo::$name {
                match self {
                    $name::NotApplicable => vo::$name::NotApplicable,
                    $($name::$variant => vo::$name::$variant,)*
                }
            }
        }
    };
}

mirror_enum!(WorkabilityRating {
    VeryPoor,
    Poor,
    Fair,
    Easy,
    Good,
    VeryGood,
    Excellent,
});

mirror_enum!(WorkabilityDefect {
    Flawless,
    Fair,
    Fluffy,
    Crushed,
    RippedOff,
    Lifted,
    Scratched,
    FluffyAndCrushed,
    FluffyAndRippedOff,
    FluffyAndScratched,
    FluffyAndRippedOffAndLifted,
    FluffyAndCrushedAndLifted,
    CrushedAndRippedOff,
    CrushedAndFluffyAndRippedOff,
    LiftedAndRippedOff,
});

mirror_enum!(WorkabilityEquipmentSettings {
    NotSuitable,
    Suitable,
});

mirror_enum!(NailCrackBehavior {
    DoesNotCrack,
    DoesNotCrackExceptCorners,
    Cracks,
});

mirror_enum!(TreatabilityRating {
    Green,
    Dry,
    VeryHard,
    Hard,
    ModeratelyHard,
    ModeratelyEasy,
    Easy,
    VeryEasy,
});

impl Convert<WoodResponse> for &Wood {
    fn convert(self) -> WoodResponse {
        let wood = self;

        WoodResponse {
            id: wood.id().to_string(),
            name: wood.name().to_string(),
            scientific_name: wood.scientific_name().to_string(),
            genus: wood.scientific_name().genus().to_owned(),
            details: wood.details().convert(),
        }
    }
}

impl Convert<WoodPageResponse> for WoodPage {
    fn convert(self) -> WoodPageResponse {
        let page = self;

        WoodPageResponse {
            items: page.items.iter().map(|wood| wood.convert()).collect(),
            total: page.total,
            offset: page.offset,
            limit: page.limit,
        }
    }
}

impl Convert<WoodDetailsSchema> for &vo::WoodDetails {
    fn convert(self) -> WoodDetailsSchema {
        let details = self;
        let description = &details.description;
        let images = &details.images;
        let density = &details.physical_properties.density;
        let shrinkage = &details.physical_properties.shrinkage;
        let workabilities = &details.workabilities;
        let equipment = &workabilities.equipment_finishing;
        let surface = &workabilities.surface_finishing;

        WoodDetailsSchema {
            family: details.family.as_ref().map(ToString::to_string),
            collection_location: details.collection_location.clone(),
            common_names: details.common_names.as_slice().to_vec(),
            source: details.source.as_ref().map(|source| SourceSchema {
                species_id: source.species_id,
                page_url: source.page_url.to_string(),
            }),
            description: DescriptionSchema {
                tree: attributes(&description.tree),
                general_characteristics: attributes(&description.general_characteristics),
                end_uses: description.end_uses.clone(),
                preservation: description.preservation.clone(),
                workability_notes: description.workability_notes.clone(),
                heartwood_color_classification: description.heartwood_color_classification.clone(),
            },
            kiln_drying: KilnDryingSchema {
                schedule: attributes(&details.kiln_drying.schedule),
                duration_days: value(details.kiln_drying.duration),
            },
            images: ImagesSchema {
                main: url(&images.main),
                tree: url(&images.tree),
                log: url(&images.log),
                bark: url(&images.bark),
                tangential_cut: url(&images.tangential_cut),
                cross_section: url(&images.cross_section),
                radial_cut: url(&images.radial_cut),
                wood: url(&images.wood),
            },
            physical_properties: PhysicalPropertiesSchema {
                density: DensitySchema {
                    dry: value(density.dry),
                    green: value(density.green),
                    basic: value(density.basic),
                    apparent: value(density.apparent),
                },
                shrinkage: ShrinkageSchema {
                    tangential: value(shrinkage.tangential),
                    radial: value(shrinkage.radial),
                    volumetric: value(shrinkage.volumetric),
                    tangential_radial_ratio: shrinkage.tangential_radial_ratio(),
                },
            },
            mechanical_properties: MechanicalPropertiesSchema {
                green: (&details.mechanical_properties.green).convert(),
                dry: (&details.mechanical_properties.dry).convert(),
            },
            workabilities: WorkabilitiesSchema {
                equipment_finishing: EquipmentFinishingSchema {
                    molding_planer: (&equipment.molding_planer).convert(),
                    narrow_belt_sander: (&equipment.narrow_belt_sander).convert(),
                    lathe: (&equipment.lathe).convert(),
                    drill_bit: (&equipment.drill_bit).convert(),
                },
                surface_finishing: SurfaceFinishingSchema {
                    planer: surface.planer.convert(),
                    sandpaper: surface.sandpaper.convert(),
                    vise: surface.vise.convert(),
                    drill: surface.drill.convert(),
                    nail_crack: surface.nail_crack.convert(),
                },
                treatabilities: TreatabilitiesSchema {
                    heartwood: workabilities.treatabilities.heartwood.convert(),
                    sapwood: workabilities.treatabilities.sapwood.convert(),
                },
            },
        }
    }
}

impl TryConvert<vo::WoodDetails> for WoodDetailsSchema {
    fn try_convert(self) -> Result<vo::WoodDetails, WoodError> {
        let schema = self;
        let description = schema.description;
        let images = schema.images;
        let density = schema.physical_properties.density;
        let shrinkage = schema.physical_properties.shrinkage;
        let workabilities = schema.workabilities;
        let equipment = workabilities.equipment_finishing;
        let surface = workabilities.surface_finishing;

        Ok(vo::WoodDetails {
            family: schema.family.map(vo::BotanicalFamily::new).transpose()?,
            collection_location: schema.collection_location,
            common_names: vo::CommonNames::new(schema.common_names),
            source: schema
                .source
                .map(|source| -> Result<_, WoodError> {
                    Ok(vo::SourceReference {
                        species_id: source.species_id,
                        page_url: vo::WebUrl::new(source.page_url)?,
                    })
                })
                .transpose()?,
            description: vo::WoodDescription {
                tree: vo::Attributes::new(description.tree),
                general_characteristics: vo::Attributes::new(description.general_characteristics),
                end_uses: description.end_uses,
                preservation: description.preservation,
                workability_notes: description.workability_notes,
                heartwood_color_classification: description.heartwood_color_classification,
            },
            kiln_drying: vo::KilnDrying {
                schedule: vo::Attributes::new(schema.kiln_drying.schedule),
                duration: Measurement::from_optional(schema.kiln_drying.duration_days)?,
            },
            images: vo::WoodImages {
                main: optional_url(images.main)?,
                tree: optional_url(images.tree)?,
                log: optional_url(images.log)?,
                bark: optional_url(images.bark)?,
                tangential_cut: optional_url(images.tangential_cut)?,
                cross_section: optional_url(images.cross_section)?,
                radial_cut: optional_url(images.radial_cut)?,
                wood: optional_url(images.wood)?,
            },
            physical_properties: vo::PhysicalProperties {
                density: vo::Density {
                    dry: Measurement::from_optional(density.dry)?,
                    green: Measurement::from_optional(density.green)?,
                    basic: Measurement::from_optional(density.basic)?,
                    apparent: Measurement::from_optional(density.apparent)?,
                },
                shrinkage: vo::Shrinkage {
                    tangential: Measurement::from_optional(shrinkage.tangential)?,
                    radial: Measurement::from_optional(shrinkage.radial)?,
                    volumetric: Measurement::from_optional(shrinkage.volumetric)?,
                },
            },
            mechanical_properties: vo::MechanicalProperties {
                green: schema.mechanical_properties.green.try_convert()?,
                dry: schema.mechanical_properties.dry.try_convert()?,
            },
            workabilities: vo::Workabilities {
                equipment_finishing: vo::EquipmentFinishing {
                    molding_planer: equipment.molding_planer.convert(),
                    narrow_belt_sander: equipment.narrow_belt_sander.convert(),
                    lathe: equipment.lathe.convert(),
                    drill_bit: equipment.drill_bit.convert(),
                },
                surface_finishing: vo::SurfaceFinishing {
                    planer: surface.planer.convert(),
                    sandpaper: surface.sandpaper.convert(),
                    vise: surface.vise.convert(),
                    drill: surface.drill.convert(),
                    nail_crack: surface.nail_crack.convert(),
                },
                treatabilities: vo::Treatabilities {
                    heartwood: workabilities.treatabilities.heartwood.convert(),
                    sapwood: workabilities.treatabilities.sapwood.convert(),
                },
            },
        })
    }
}

impl Convert<MechanicalPropertiesByConditionSchema> for &vo::MechanicalPropertiesByCondition {
    fn convert(self) -> MechanicalPropertiesByConditionSchema {
        let properties = self;

        MechanicalPropertiesByConditionSchema {
            static_bending: StaticBendingSchema {
                modulus_of_rupture: value(properties.static_bending.modulus_of_rupture),
                modulus_of_elasticity: value(properties.static_bending.modulus_of_elasticity),
            },
            compression: GrainDirectionsSchema {
                parallel_to_grain: value(properties.compression.parallel_to_grain),
                perpendicular_to_grain: value(properties.compression.perpendicular_to_grain),
            },
            janka_hardness: GrainForceSchema {
                parallel_to_grain: value(properties.janka_hardness.parallel_to_grain),
                end_grain: value(properties.janka_hardness.end_grain),
            },
            tension_perpendicular_to_grain: value(properties.tension_perpendicular_to_grain),
            cleavage_strength: value(properties.cleavage_strength),
            shear_parallel_to_grain: value(properties.shear_parallel_to_grain),
            nail_extraction: GrainForceSchema {
                parallel_to_grain: value(properties.nail_extraction.parallel_to_grain),
                end_grain: value(properties.nail_extraction.end_grain),
            },
        }
    }
}

impl TryConvert<vo::MechanicalPropertiesByCondition> for MechanicalPropertiesByConditionSchema {
    fn try_convert(self) -> Result<vo::MechanicalPropertiesByCondition, WoodError> {
        let schema = self;

        Ok(vo::MechanicalPropertiesByCondition {
            static_bending: vo::StaticBending {
                modulus_of_rupture: Measurement::from_optional(
                    schema.static_bending.modulus_of_rupture,
                )?,
                modulus_of_elasticity: Measurement::from_optional(
                    schema.static_bending.modulus_of_elasticity,
                )?,
            },
            compression: vo::Compression {
                parallel_to_grain: Measurement::from_optional(
                    schema.compression.parallel_to_grain,
                )?,
                perpendicular_to_grain: Measurement::from_optional(
                    schema.compression.perpendicular_to_grain,
                )?,
            },
            janka_hardness: vo::JankaHardness {
                parallel_to_grain: Measurement::from_optional(
                    schema.janka_hardness.parallel_to_grain,
                )?,
                end_grain: Measurement::from_optional(schema.janka_hardness.end_grain)?,
            },
            tension_perpendicular_to_grain: Measurement::from_optional(
                schema.tension_perpendicular_to_grain,
            )?,
            cleavage_strength: Measurement::from_optional(schema.cleavage_strength)?,
            shear_parallel_to_grain: Measurement::from_optional(schema.shear_parallel_to_grain)?,
            nail_extraction: vo::NailExtraction {
                parallel_to_grain: Measurement::from_optional(
                    schema.nail_extraction.parallel_to_grain,
                )?,
                end_grain: Measurement::from_optional(schema.nail_extraction.end_grain)?,
            },
        })
    }
}

impl Convert<EquipmentFinishingResultSchema> for &vo::EquipmentFinishingResult {
    fn convert(self) -> EquipmentFinishingResultSchema {
        let result = self;

        EquipmentFinishingResultSchema {
            number_of_samples: result.number_of_samples,
            surface_finish: result.surface_finish.convert(),
            defect: result.defect.convert(),
            equipment_settings: result.equipment_settings.convert(),
        }
    }
}

impl Convert<vo::EquipmentFinishingResult> for EquipmentFinishingResultSchema {
    fn convert(self) -> vo::EquipmentFinishingResult {
        let schema = self;

        vo::EquipmentFinishingResult {
            number_of_samples: schema.number_of_samples,
            surface_finish: schema.surface_finish.convert(),
            defect: schema.defect.convert(),
            equipment_settings: schema.equipment_settings.convert(),
        }
    }
}

fn value<M: Measurement>(measurement: Option<M>) -> Option<f64> {
    measurement.map(|measurement| measurement.value())
}

fn url(url: &Option<vo::WebUrl>) -> Option<String> {
    url.as_ref().map(ToString::to_string)
}

fn optional_url(url: Option<String>) -> Result<Option<vo::WebUrl>, WoodError> {
    url.map(vo::WebUrl::new).transpose()
}

fn attributes(attributes: &vo::Attributes) -> BTreeMap<String, String> {
    attributes
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use zylotek_domain::testing::{full_details, wood};

    use super::*;

    fn round_trip(details: &vo::WoodDetails) -> vo::WoodDetails {
        let schema: WoodDetailsSchema = details.convert();
        let json = serde_json::to_value(&schema).unwrap();

        serde_json::from_value::<WoodDetailsSchema>(json)
            .unwrap()
            .try_convert()
            .unwrap()
    }

    fn to_domain(value: serde_json::Value) -> Result<vo::WoodDetails, WoodError> {
        serde_json::from_value::<WoodDetailsSchema>(value)
            .unwrap()
            .try_convert()
    }

    #[test]
    fn full_details_survive_schema_and_json() {
        assert_eq!(round_trip(&full_details()), full_details());
    }

    #[test]
    fn empty_details_survive_schema_and_json() {
        assert_eq!(
            round_trip(&vo::WoodDetails::default()),
            vo::WoodDetails::default()
        );
    }

    #[test]
    fn empty_json_object_converts_to_empty_details() {
        assert_eq!(to_domain(json!({})).unwrap(), vo::WoodDetails::default());
    }

    #[test]
    fn the_ratio_is_computed_by_the_server_whatever_a_client_sends() {
        let sent = json!({"physical_properties": {"shrinkage": {
            "tangential": 6.0, "radial": 3.0, "tangential_radial_ratio": 99.0
        }}});

        let stored = to_domain(sent).unwrap();
        let echoed: WoodDetailsSchema = (&stored).convert();

        assert_eq!(
            echoed.physical_properties.shrinkage.tangential_radial_ratio,
            Some(2.0)
        );
        let full: WoodResponse = (&wood("Aroeira", "Myracrodruon urundeuva")).convert();
        assert_eq!(
            full.details
                .physical_properties
                .shrinkage
                .tangential_radial_ratio,
            Some(1.5)
        );
    }

    #[test]
    fn the_response_carries_identity_and_genus() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");

        let response: WoodResponse = (&stored).convert();

        assert_eq!(response.id, stored.id().to_string());
        assert_eq!(response.name, "Aroeira");
        assert_eq!(response.scientific_name, "Myracrodruon urundeuva");
        assert_eq!(response.genus, "Myracrodruon");
    }

    #[test]
    fn the_page_response_mirrors_the_page() {
        let page = WoodPage {
            items: vec![wood("Aroeira", "Myracrodruon urundeuva")],
            total: 7,
            offset: 2,
            limit: 5,
        };

        let response: WoodPageResponse = page.convert();

        assert_eq!((response.total, response.offset, response.limit), (7, 2, 5));
        assert_eq!(response.items.len(), 1);
    }

    #[test]
    fn invalid_values_are_rejected_wherever_they_appear() {
        let invalid = [
            json!({"family": " "}),
            json!({"source": {"species_id": 1, "page_url": "nope"}}),
            json!({"kiln_drying": {"duration_days": -1.0}}),
            json!({"images": {"wood": "relative/path.jpg"}}),
            json!({"physical_properties": {"density": {"green": -0.1}}}),
            json!({"physical_properties": {"shrinkage": {"volumetric": 100.5}}}),
            json!({"mechanical_properties": {"dry": {"static_bending": {"modulus_of_rupture": -1.0}}}}),
            json!({"mechanical_properties": {"dry": {"static_bending": {"modulus_of_elasticity": -1.0}}}}),
            json!({"mechanical_properties": {"green": {"compression": {"parallel_to_grain": -1.0}}}}),
            json!({"mechanical_properties": {"green": {"compression": {"perpendicular_to_grain": -1.0}}}}),
            json!({"mechanical_properties": {"green": {"janka_hardness": {"parallel_to_grain": -1.0}}}}),
            json!({"mechanical_properties": {"green": {"janka_hardness": {"end_grain": -1.0}}}}),
            json!({"mechanical_properties": {"green": {"nail_extraction": {"parallel_to_grain": -1.0}}}}),
            json!({"mechanical_properties": {"green": {"tension_perpendicular_to_grain": -1.0}}}),
            json!({"mechanical_properties": {"green": {"cleavage_strength": -1.0}}}),
            json!({"mechanical_properties": {"green": {"shear_parallel_to_grain": -1.0}}}),
        ];

        for value in invalid {
            assert!(
                to_domain(value.clone()).is_err(),
                "{value} must be rejected"
            );
        }
    }

    /// Every variant goes domain -> wire -> domain unchanged.
    macro_rules! assert_enum_round_trips {
        ($name:ident: $($variant:ident),+ $(,)?) => {
            $(
                let wire: $name = vo::$name::$variant.convert();
                let back: vo::$name = wire.convert();
                assert_eq!(back, vo::$name::$variant);
            )+
        };
    }

    #[test]
    fn every_enum_variant_survives_the_round_trip() {
        assert_enum_round_trips!(WorkabilityRating: NotApplicable, VeryPoor, Poor, Fair, Easy, Good, VeryGood, Excellent);
        assert_enum_round_trips!(WorkabilityDefect: NotApplicable, Flawless, Fair, Fluffy, Crushed, RippedOff, Lifted, Scratched, FluffyAndCrushed, FluffyAndRippedOff, FluffyAndScratched, FluffyAndRippedOffAndLifted, FluffyAndCrushedAndLifted, CrushedAndRippedOff, CrushedAndFluffyAndRippedOff, LiftedAndRippedOff);
        assert_enum_round_trips!(WorkabilityEquipmentSettings: NotApplicable, NotSuitable, Suitable);
        assert_enum_round_trips!(NailCrackBehavior: NotApplicable, DoesNotCrack, DoesNotCrackExceptCorners, Cracks);
        assert_enum_round_trips!(TreatabilityRating: NotApplicable, Green, Dry, VeryHard, Hard, ModeratelyHard, ModeratelyEasy, Easy, VeryEasy);
    }
}
