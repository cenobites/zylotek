use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zylotek_application::woods::list_woods::WoodPage;
use zylotek_domain::woods::{
    entities::Wood,
    errors::WoodError,
    value_objects::{self as vo, Measurement},
};

/// Body of `POST /woods` and `PUT /woods/{id}`. Everything but the names is optional.
#[derive(Debug, Deserialize)]
pub struct WoodRequest {
    pub name: String,
    pub scientific_name: String,
    #[serde(flatten)]
    pub details: WoodDetailsSchema,
}

#[derive(Debug, Serialize)]
pub struct WoodResponse {
    pub id: String,
    pub name: String,
    pub scientific_name: String,
    pub genus: String,
    #[serde(flatten)]
    pub details: WoodDetailsSchema,
}

#[derive(Debug, Serialize)]
pub struct WoodPageResponse {
    pub items: Vec<WoodResponse>,
    pub total: u64,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Deserialize)]
pub struct ListWoodsParams {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WoodDetailsSchema {
    pub family: Option<String>,
    pub collection_location: Option<String>,
    pub common_names: Vec<String>,
    pub source: Option<SourceSchema>,
    pub description: DescriptionSchema,
    pub kiln_drying: KilnDryingSchema,
    pub images: ImagesSchema,
    pub physical_properties: PhysicalPropertiesSchema,
    pub mechanical_properties: MechanicalPropertiesSchema,
    pub workabilities: WorkabilitiesSchema,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SourceSchema {
    pub species_id: u64,
    pub page_url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DescriptionSchema {
    pub tree: BTreeMap<String, String>,
    pub general_characteristics: BTreeMap<String, String>,
    pub end_uses: Option<String>,
    pub preservation: Option<String>,
    pub workability_notes: Option<String>,
    pub heartwood_color_classification: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KilnDryingSchema {
    pub schedule: BTreeMap<String, String>,
    pub duration_days: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ImagesSchema {
    pub main: Option<String>,
    pub tree: Option<String>,
    pub log: Option<String>,
    pub bark: Option<String>,
    pub tangential_cut: Option<String>,
    pub cross_section: Option<String>,
    pub radial_cut: Option<String>,
    pub wood: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PhysicalPropertiesSchema {
    /// g/cm³
    pub density: DensitySchema,
    /// %
    pub shrinkage: ShrinkageSchema,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DensitySchema {
    pub dry: Option<f64>,
    pub green: Option<f64>,
    pub basic: Option<f64>,
    pub apparent: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ShrinkageSchema {
    pub tangential: Option<f64>,
    pub radial: Option<f64>,
    pub volumetric: Option<f64>,
    /// Computed from tangential and radial; ignored on input.
    #[serde(skip_deserializing)]
    pub tangential_radial_ratio: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MechanicalPropertiesSchema {
    pub green: MechanicalPropertiesByConditionSchema,
    pub dry: MechanicalPropertiesByConditionSchema,
}

/// Stresses in kgf/cm², cleavage in kgf/cm, hardness and nail extraction in kgf.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MechanicalPropertiesByConditionSchema {
    pub static_bending: StaticBendingSchema,
    pub compression: GrainDirectionsSchema,
    pub janka_hardness: GrainForceSchema,
    pub tension_perpendicular_to_grain: Option<f64>,
    pub cleavage_strength: Option<f64>,
    pub shear_parallel_to_grain: Option<f64>,
    pub nail_extraction: GrainForceSchema,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StaticBendingSchema {
    pub modulus_of_rupture: Option<f64>,
    pub modulus_of_elasticity: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GrainDirectionsSchema {
    pub parallel_to_grain: Option<f64>,
    pub perpendicular_to_grain: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GrainForceSchema {
    pub parallel_to_grain: Option<f64>,
    pub end_grain: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkabilitiesSchema {
    pub equipment_finishing: EquipmentFinishingSchema,
    pub surface_finishing: SurfaceFinishingSchema,
    pub treatabilities: TreatabilitiesSchema,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EquipmentFinishingSchema {
    pub molding_planer: EquipmentFinishingResultSchema,
    pub narrow_belt_sander: EquipmentFinishingResultSchema,
    pub lathe: EquipmentFinishingResultSchema,
    pub drill_bit: EquipmentFinishingResultSchema,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EquipmentFinishingResultSchema {
    pub number_of_samples: Option<u32>,
    pub surface_finish: WorkabilityRating,
    pub defect: WorkabilityDefect,
    pub equipment_settings: WorkabilityEquipmentSettings,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfaceFinishingSchema {
    pub planer: WorkabilityRating,
    pub sandpaper: WorkabilityRating,
    pub vise: WorkabilityRating,
    pub drill: WorkabilityRating,
    pub nail_crack: NailCrackBehavior,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TreatabilitiesSchema {
    pub heartwood: TreatabilityRating,
    pub sapwood: TreatabilityRating,
}

/// Declares a snake_case JSON enum mirroring a domain enum, with conversions both ways.
macro_rules! schema_enum {
    ($name:ident { $($variant:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            #[default]
            NotApplicable,
            $($variant,)*
        }

        impl From<vo::$name> for $name {
            fn from(value: vo::$name) -> Self {
                match value {
                    vo::$name::NotApplicable => Self::NotApplicable,
                    $(vo::$name::$variant => Self::$variant,)*
                }
            }
        }

        impl From<$name> for vo::$name {
            fn from(value: $name) -> Self {
                match value {
                    $name::NotApplicable => Self::NotApplicable,
                    $($name::$variant => Self::$variant,)*
                }
            }
        }
    };
}

schema_enum!(WorkabilityRating {
    VeryPoor,
    Poor,
    Fair,
    Easy,
    Good,
    VeryGood,
    Excellent,
});

schema_enum!(WorkabilityDefect {
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

schema_enum!(WorkabilityEquipmentSettings {
    NotSuitable,
    Suitable,
});

schema_enum!(NailCrackBehavior {
    DoesNotCrack,
    DoesNotCrackExceptCorners,
    Cracks,
});

schema_enum!(TreatabilityRating {
    Green,
    Dry,
    VeryHard,
    Hard,
    ModeratelyHard,
    ModeratelyEasy,
    Easy,
    VeryEasy,
});

impl From<&Wood> for WoodResponse {
    fn from(wood: &Wood) -> Self {
        Self {
            id: wood.id().to_string(),
            name: wood.name().to_string(),
            scientific_name: wood.scientific_name().to_string(),
            genus: wood.scientific_name().genus().to_owned(),
            details: wood.details().into(),
        }
    }
}

impl From<WoodPage> for WoodPageResponse {
    fn from(page: WoodPage) -> Self {
        Self {
            items: page.items.iter().map(WoodResponse::from).collect(),
            total: page.total,
            offset: page.offset,
            limit: page.limit,
        }
    }
}

impl From<&vo::WoodDetails> for WoodDetailsSchema {
    fn from(details: &vo::WoodDetails) -> Self {
        let description = &details.description;
        let images = &details.images;
        let density = &details.physical_properties.density;
        let shrinkage = &details.physical_properties.shrinkage;
        let workabilities = &details.workabilities;
        let equipment = &workabilities.equipment_finishing;
        let surface = &workabilities.surface_finishing;

        Self {
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
                green: (&details.mechanical_properties.green).into(),
                dry: (&details.mechanical_properties.dry).into(),
            },
            workabilities: WorkabilitiesSchema {
                equipment_finishing: EquipmentFinishingSchema {
                    molding_planer: (&equipment.molding_planer).into(),
                    narrow_belt_sander: (&equipment.narrow_belt_sander).into(),
                    lathe: (&equipment.lathe).into(),
                    drill_bit: (&equipment.drill_bit).into(),
                },
                surface_finishing: SurfaceFinishingSchema {
                    planer: surface.planer.into(),
                    sandpaper: surface.sandpaper.into(),
                    vise: surface.vise.into(),
                    drill: surface.drill.into(),
                    nail_crack: surface.nail_crack.into(),
                },
                treatabilities: TreatabilitiesSchema {
                    heartwood: workabilities.treatabilities.heartwood.into(),
                    sapwood: workabilities.treatabilities.sapwood.into(),
                },
            },
        }
    }
}

impl TryFrom<WoodDetailsSchema> for vo::WoodDetails {
    type Error = WoodError;

    fn try_from(schema: WoodDetailsSchema) -> Result<Self, Self::Error> {
        let description = schema.description;
        let images = schema.images;
        let density = schema.physical_properties.density;
        let shrinkage = schema.physical_properties.shrinkage;
        let workabilities = schema.workabilities;
        let equipment = workabilities.equipment_finishing;
        let surface = workabilities.surface_finishing;

        Ok(Self {
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
                green: schema.mechanical_properties.green.try_into()?,
                dry: schema.mechanical_properties.dry.try_into()?,
            },
            workabilities: vo::Workabilities {
                equipment_finishing: vo::EquipmentFinishing {
                    molding_planer: equipment.molding_planer.into(),
                    narrow_belt_sander: equipment.narrow_belt_sander.into(),
                    lathe: equipment.lathe.into(),
                    drill_bit: equipment.drill_bit.into(),
                },
                surface_finishing: vo::SurfaceFinishing {
                    planer: surface.planer.into(),
                    sandpaper: surface.sandpaper.into(),
                    vise: surface.vise.into(),
                    drill: surface.drill.into(),
                    nail_crack: surface.nail_crack.into(),
                },
                treatabilities: vo::Treatabilities {
                    heartwood: workabilities.treatabilities.heartwood.into(),
                    sapwood: workabilities.treatabilities.sapwood.into(),
                },
            },
        })
    }
}

impl From<&vo::MechanicalPropertiesByCondition> for MechanicalPropertiesByConditionSchema {
    fn from(properties: &vo::MechanicalPropertiesByCondition) -> Self {
        Self {
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

impl TryFrom<MechanicalPropertiesByConditionSchema> for vo::MechanicalPropertiesByCondition {
    type Error = WoodError;

    fn try_from(schema: MechanicalPropertiesByConditionSchema) -> Result<Self, Self::Error> {
        Ok(Self {
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

impl From<&vo::EquipmentFinishingResult> for EquipmentFinishingResultSchema {
    fn from(result: &vo::EquipmentFinishingResult) -> Self {
        Self {
            number_of_samples: result.number_of_samples,
            surface_finish: result.surface_finish.into(),
            defect: result.defect.into(),
            equipment_settings: result.equipment_settings.into(),
        }
    }
}

impl From<EquipmentFinishingResultSchema> for vo::EquipmentFinishingResult {
    fn from(schema: EquipmentFinishingResultSchema) -> Self {
        Self {
            number_of_samples: schema.number_of_samples,
            surface_finish: schema.surface_finish.into(),
            defect: schema.defect.into(),
            equipment_settings: schema.equipment_settings.into(),
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
        let schema = WoodDetailsSchema::from(details);
        let json = serde_json::to_value(&schema).unwrap();

        serde_json::from_value::<WoodDetailsSchema>(json)
            .unwrap()
            .try_into()
            .unwrap()
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
    fn empty_json_object_deserializes_to_empty_details() {
        let schema: WoodDetailsSchema = serde_json::from_value(json!({})).unwrap();

        assert_eq!(
            vo::WoodDetails::try_from(schema).unwrap(),
            vo::WoodDetails::default()
        );
    }

    #[test]
    fn the_request_flattens_details_next_to_the_names() {
        let request: WoodRequest = serde_json::from_value(json!({
            "name": "Aroeira",
            "scientific_name": "Myracrodruon urundeuva",
            "family": "Anacardiaceae",
            "kiln_drying": {"duration_days": 3.5},
        }))
        .unwrap();

        assert_eq!(request.name, "Aroeira");
        assert_eq!(request.details.family.as_deref(), Some("Anacardiaceae"));
        assert_eq!(request.details.kiln_drying.duration_days, Some(3.5));
    }

    #[test]
    fn the_computed_ratio_is_ignored_on_input_and_emitted_on_output() {
        let input: ShrinkageSchema = serde_json::from_value(
            json!({"tangential": 6.0, "radial": 3.0, "tangential_radial_ratio": 99.0}),
        )
        .unwrap();
        assert_eq!(input.tangential_radial_ratio, None);

        let response = WoodResponse::from(&wood("Aroeira", "Myracrodruon urundeuva"));
        assert_eq!(
            response
                .details
                .physical_properties
                .shrinkage
                .tangential_radial_ratio,
            Some(1.5)
        );
    }

    #[test]
    fn the_response_carries_identity_and_genus() {
        let stored = wood("Aroeira", "Myracrodruon urundeuva");

        let response = WoodResponse::from(&stored);

        assert_eq!(response.id, stored.id().to_string());
        assert_eq!(response.name, "Aroeira");
        assert_eq!(response.genus, "Myracrodruon");
        assert!(format!("{response:?}").contains("Aroeira"));
    }

    #[test]
    fn the_page_response_mirrors_the_page() {
        let page = WoodPage {
            items: vec![wood("Aroeira", "Myracrodruon urundeuva")],
            total: 7,
            offset: 2,
            limit: 5,
        };

        let response = WoodPageResponse::from(page);

        assert_eq!((response.total, response.offset, response.limit), (7, 2, 5));
        assert_eq!(response.items.len(), 1);
        assert!(format!("{response:?}").contains("items"));
    }

    #[test]
    fn list_params_are_all_optional() {
        let params: ListWoodsParams = serde_json::from_value(json!({})).unwrap();

        assert_eq!((params.offset, params.limit), (None, None));
        assert!(format!("{params:?}").contains("offset"));
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
            let schema: WoodDetailsSchema = serde_json::from_value(value.clone()).unwrap();

            assert!(
                vo::WoodDetails::try_from(schema).is_err(),
                "{value} must be rejected"
            );
        }
    }

    #[test]
    fn unknown_enum_names_fail_to_deserialize() {
        let error =
            serde_json::from_value::<TreatabilitiesSchema>(json!({"heartwood": "excellent"}));

        assert!(error.is_err());
    }

    /// Every variant converts to the domain and back, and serializes in snake_case.
    macro_rules! assert_schema_enum {
        ($name:ident: $($variant:ident => $json:literal),+ $(,)?) => {
            $(
                let schema = $name::from(vo::$name::$variant);
                assert_eq!(vo::$name::from(schema), vo::$name::$variant);
                assert_eq!(serde_json::to_value(schema).unwrap(), json!($json));
                assert_eq!(format!("{:?}", serde_json::from_value::<$name>(json!($json)).unwrap()), format!("{schema:?}"));
            )+
        };
    }

    #[test]
    fn every_enum_variant_converts_and_uses_snake_case() {
        assert_schema_enum!(WorkabilityRating:
            NotApplicable => "not_applicable", VeryPoor => "very_poor", Poor => "poor", Fair => "fair",
            Easy => "easy", Good => "good", VeryGood => "very_good", Excellent => "excellent");
        assert_schema_enum!(WorkabilityDefect:
            NotApplicable => "not_applicable", Flawless => "flawless", Fair => "fair", Fluffy => "fluffy",
            Crushed => "crushed", RippedOff => "ripped_off", Lifted => "lifted", Scratched => "scratched",
            FluffyAndCrushed => "fluffy_and_crushed", FluffyAndRippedOff => "fluffy_and_ripped_off",
            FluffyAndScratched => "fluffy_and_scratched",
            FluffyAndRippedOffAndLifted => "fluffy_and_ripped_off_and_lifted",
            FluffyAndCrushedAndLifted => "fluffy_and_crushed_and_lifted",
            CrushedAndRippedOff => "crushed_and_ripped_off",
            CrushedAndFluffyAndRippedOff => "crushed_and_fluffy_and_ripped_off",
            LiftedAndRippedOff => "lifted_and_ripped_off");
        assert_schema_enum!(WorkabilityEquipmentSettings:
            NotApplicable => "not_applicable", NotSuitable => "not_suitable", Suitable => "suitable");
        assert_schema_enum!(NailCrackBehavior:
            NotApplicable => "not_applicable", DoesNotCrack => "does_not_crack",
            DoesNotCrackExceptCorners => "does_not_crack_except_corners", Cracks => "cracks");
        assert_schema_enum!(TreatabilityRating:
            NotApplicable => "not_applicable", Green => "green", Dry => "dry", VeryHard => "very_hard",
            Hard => "hard", ModeratelyHard => "moderately_hard", ModeratelyEasy => "moderately_easy",
            Easy => "easy", VeryEasy => "very_easy");
    }

    #[test]
    fn enums_default_to_not_applicable() {
        assert_eq!(
            serde_json::to_value(WorkabilityRating::default()).unwrap(),
            json!("not_applicable")
        );
        assert_eq!(
            serde_json::to_value(NailCrackBehavior::default()).unwrap(),
            json!("not_applicable")
        );
    }
}
