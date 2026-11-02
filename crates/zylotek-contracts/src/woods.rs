//! The JSON shapes of the wood API: what `zylotek-api` accepts and returns, and what its
//! clients (the web app, the assistant) read. Plain data with serde derives; converting to and
//! from the domain is the API's job.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Body of `POST /woods` and `PUT /woods/{id}`. Everything but the names is optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WoodRequest {
    pub name: String,
    pub scientific_name: String,
    #[serde(flatten)]
    pub details: WoodDetailsSchema,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WoodResponse {
    pub id: String,
    pub name: String,
    pub scientific_name: String,
    pub genus: String,
    #[serde(flatten)]
    pub details: WoodDetailsSchema,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WoodPageResponse {
    pub items: Vec<WoodResponse>,
    pub total: u64,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListWoodsParams {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceSchema {
    pub species_id: u64,
    pub page_url: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DescriptionSchema {
    pub tree: BTreeMap<String, String>,
    pub general_characteristics: BTreeMap<String, String>,
    pub end_uses: Option<String>,
    pub preservation: Option<String>,
    pub workability_notes: Option<String>,
    pub heartwood_color_classification: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KilnDryingSchema {
    pub schedule: BTreeMap<String, String>,
    pub duration_days: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PhysicalPropertiesSchema {
    /// g/cm³
    pub density: DensitySchema,
    /// %
    pub shrinkage: ShrinkageSchema,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DensitySchema {
    pub dry: Option<f64>,
    pub green: Option<f64>,
    pub basic: Option<f64>,
    pub apparent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ShrinkageSchema {
    pub tangential: Option<f64>,
    pub radial: Option<f64>,
    pub volumetric: Option<f64>,
    /// Computed by the server from tangential and radial; ignored when sent in a request.
    pub tangential_radial_ratio: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MechanicalPropertiesSchema {
    pub green: MechanicalPropertiesByConditionSchema,
    pub dry: MechanicalPropertiesByConditionSchema,
}

/// Stresses in kgf/cm², cleavage in kgf/cm, hardness and nail extraction in kgf.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StaticBendingSchema {
    pub modulus_of_rupture: Option<f64>,
    pub modulus_of_elasticity: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GrainDirectionsSchema {
    pub parallel_to_grain: Option<f64>,
    pub perpendicular_to_grain: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GrainForceSchema {
    pub parallel_to_grain: Option<f64>,
    pub end_grain: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkabilitiesSchema {
    pub equipment_finishing: EquipmentFinishingSchema,
    pub surface_finishing: SurfaceFinishingSchema,
    pub treatabilities: TreatabilitiesSchema,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EquipmentFinishingSchema {
    pub molding_planer: EquipmentFinishingResultSchema,
    pub narrow_belt_sander: EquipmentFinishingResultSchema,
    pub lathe: EquipmentFinishingResultSchema,
    pub drill_bit: EquipmentFinishingResultSchema,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EquipmentFinishingResultSchema {
    pub number_of_samples: Option<u32>,
    pub surface_finish: WorkabilityRating,
    pub defect: WorkabilityDefect,
    pub equipment_settings: WorkabilityEquipmentSettings,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfaceFinishingSchema {
    pub planer: WorkabilityRating,
    pub sandpaper: WorkabilityRating,
    pub vise: WorkabilityRating,
    pub drill: WorkabilityRating,
    pub nail_crack: NailCrackBehavior,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TreatabilitiesSchema {
    pub heartwood: TreatabilityRating,
    pub sapwood: TreatabilityRating,
}

/// Declares a snake_case JSON enum for a domain rating. `NotApplicable` is always first.
macro_rules! schema_enum {
    ($name:ident { $($variant:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            #[default]
            NotApplicable,
            $($variant,)*
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

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// A wood response with every field set: the wire format, written out.
    fn full_response() -> Value {
        let by_condition = |scale: f64| {
            json!({
                "static_bending": {"modulus_of_rupture": 1000.0 * scale, "modulus_of_elasticity": 120000.0 * scale},
                "compression": {"parallel_to_grain": 500.0 * scale, "perpendicular_to_grain": 90.0 * scale},
                "janka_hardness": {"parallel_to_grain": 700.0 * scale, "end_grain": 800.0 * scale},
                "tension_perpendicular_to_grain": 60.0 * scale,
                "cleavage_strength": 70.0 * scale,
                "shear_parallel_to_grain": 110.0 * scale,
                "nail_extraction": {"parallel_to_grain": 150.0 * scale, "end_grain": 120.0 * scale},
            })
        };
        let equipment = |rating: &str, defect: &str, settings: &str| json!({"number_of_samples": 10, "surface_finish": rating, "defect": defect, "equipment_settings": settings});

        json!({
            "id": "6f1b6c4e-0000-4000-8000-000000000001",
            "name": "Aroeira",
            "scientific_name": "Myracrodruon urundeuva",
            "genus": "Myracrodruon",
            "family": "Anacardiaceae",
            "collection_location": "Belém, PA",
            "common_names": ["Aroeira", "Gonçalo-alves"],
            "source": {"species_id": 42, "page_url": "https://lpf.florestal.gov.br/pt-br/bd-madeiras-brasileiras"},
            "description": {
                "tree": {"altura": "30 m"},
                "general_characteristics": {"cor": "castanho"},
                "end_uses": "Construção civil",
                "preservation": "Alta durabilidade",
                "workability_notes": "Fácil de trabalhar",
                "heartwood_color_classification": "marrom",
            },
            "kiln_drying": {"schedule": {"programa": "suave"}, "duration_days": 12.5},
            "images": {
                "main": "https://x.y/main.jpg", "tree": "https://x.y/tree.jpg", "log": "https://x.y/log.jpg",
                "bark": "https://x.y/bark.jpg", "tangential_cut": "https://x.y/t.jpg",
                "cross_section": "https://x.y/c.jpg", "radial_cut": "https://x.y/r.jpg", "wood": "https://x.y/w.jpg",
            },
            "physical_properties": {
                "density": {"dry": 0.9, "green": 1.1, "basic": 0.8, "apparent": 0.95},
                "shrinkage": {"tangential": 7.5, "radial": 5.0, "volumetric": 12.5, "tangential_radial_ratio": 1.5},
            },
            "mechanical_properties": {"green": by_condition(1.0), "dry": by_condition(1.25)},
            "workabilities": {
                "equipment_finishing": {
                    "molding_planer": equipment("good", "flawless", "suitable"),
                    "narrow_belt_sander": equipment("excellent", "fluffy", "not_suitable"),
                    "lathe": equipment("fair", "crushed_and_ripped_off", "suitable"),
                    "drill_bit": equipment("very_poor", "lifted_and_ripped_off", "not_applicable"),
                },
                "surface_finishing": {
                    "planer": "very_good", "sandpaper": "easy", "vise": "poor", "drill": "good",
                    "nail_crack": "does_not_crack_except_corners",
                },
                "treatabilities": {"heartwood": "moderately_hard", "sapwood": "very_easy"},
            },
        })
    }

    #[test]
    fn a_full_response_round_trips_without_losing_or_renaming_a_field() {
        let wire = full_response();

        let response: WoodResponse = serde_json::from_value(wire.clone()).unwrap();

        assert_eq!(serde_json::to_value(&response).unwrap(), wire);
        assert_eq!(response.clone(), response);
        assert_eq!(response.genus, "Myracrodruon");
        assert_eq!(
            response
                .details
                .physical_properties
                .shrinkage
                .tangential_radial_ratio,
            Some(1.5)
        );
        assert!(format!("{response:?}").contains("Aroeira"));
    }

    #[test]
    fn a_page_wraps_responses_with_its_paging() {
        let wire = json!({"items": [full_response()], "total": 225, "offset": 100, "limit": 100});

        let page: WoodPageResponse = serde_json::from_value(wire.clone()).unwrap();

        assert_eq!((page.total, page.offset, page.limit), (225, 100, 100));
        assert_eq!(page.items.len(), 1);
        assert_eq!(serde_json::to_value(&page).unwrap(), wire);
        assert!(format!("{page:?}").contains("items"));
    }

    #[test]
    fn every_detail_is_optional_in_a_request() {
        let request: WoodRequest = serde_json::from_value(json!({
            "name": "Aroeira",
            "scientific_name": "Myracrodruon urundeuva",
        }))
        .unwrap();

        assert_eq!(request.details, WoodDetailsSchema::default());
        assert!(request.details.common_names.is_empty());
        assert_eq!(request.details.source, None);
    }

    #[test]
    fn a_request_carries_its_details_next_to_the_names() {
        let request: WoodRequest = serde_json::from_value(json!({
            "name": "Aroeira",
            "scientific_name": "Myracrodruon urundeuva",
            "family": "Anacardiaceae",
            "kiln_drying": {"duration_days": 3.5},
        }))
        .unwrap();

        assert_eq!(request.details.family.as_deref(), Some("Anacardiaceae"));
        assert_eq!(request.details.kiln_drying.duration_days, Some(3.5));
        let sent = serde_json::to_value(&request).unwrap();
        assert_eq!(sent["name"], "Aroeira");
        assert_eq!(sent["family"], "Anacardiaceae");
        assert!(format!("{request:?}").contains("Aroeira"));
    }

    #[test]
    fn unknown_fields_are_ignored_so_the_api_can_grow() {
        let response: WoodDetailsSchema =
            serde_json::from_value(json!({"family": "X", "brand_new_field": [1, 2, 3]})).unwrap();

        assert_eq!(response.family.as_deref(), Some("X"));
    }

    #[test]
    fn list_params_are_all_optional_and_serialize_for_queries() {
        let empty: ListWoodsParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!((empty.offset, empty.limit), (None, None));

        let params = ListWoodsParams {
            offset: Some(100),
            limit: Some(50),
        };
        assert_eq!(
            serde_json::to_value(&params).unwrap(),
            json!({"offset": 100, "limit": 50})
        );
        assert!(format!("{params:?}").contains("offset"));
    }

    /// Every variant serializes in snake_case and reads back.
    macro_rules! assert_wire_names {
        ($name:ident: $($variant:ident => $json:literal),+ $(,)?) => {
            $(
                assert_eq!(serde_json::to_value($name::$variant).unwrap(), json!($json));
                assert_eq!(serde_json::from_value::<$name>(json!($json)).unwrap(), $name::$variant);
            )+
        };
    }

    #[test]
    fn every_enum_variant_is_snake_case_on_the_wire() {
        assert_wire_names!(WorkabilityRating:
            NotApplicable => "not_applicable", VeryPoor => "very_poor", Poor => "poor", Fair => "fair",
            Easy => "easy", Good => "good", VeryGood => "very_good", Excellent => "excellent");
        assert_wire_names!(WorkabilityDefect:
            NotApplicable => "not_applicable", Flawless => "flawless", Fair => "fair", Fluffy => "fluffy",
            Crushed => "crushed", RippedOff => "ripped_off", Lifted => "lifted", Scratched => "scratched",
            FluffyAndCrushed => "fluffy_and_crushed", FluffyAndRippedOff => "fluffy_and_ripped_off",
            FluffyAndScratched => "fluffy_and_scratched",
            FluffyAndRippedOffAndLifted => "fluffy_and_ripped_off_and_lifted",
            FluffyAndCrushedAndLifted => "fluffy_and_crushed_and_lifted",
            CrushedAndRippedOff => "crushed_and_ripped_off",
            CrushedAndFluffyAndRippedOff => "crushed_and_fluffy_and_ripped_off",
            LiftedAndRippedOff => "lifted_and_ripped_off");
        assert_wire_names!(WorkabilityEquipmentSettings:
            NotApplicable => "not_applicable", NotSuitable => "not_suitable", Suitable => "suitable");
        assert_wire_names!(NailCrackBehavior:
            NotApplicable => "not_applicable", DoesNotCrack => "does_not_crack",
            DoesNotCrackExceptCorners => "does_not_crack_except_corners", Cracks => "cracks");
        assert_wire_names!(TreatabilityRating:
            NotApplicable => "not_applicable", Green => "green", Dry => "dry", VeryHard => "very_hard",
            Hard => "hard", ModeratelyHard => "moderately_hard", ModeratelyEasy => "moderately_easy",
            Easy => "easy", VeryEasy => "very_easy");
    }

    #[test]
    fn enums_default_to_not_applicable_and_reject_unknown_names() {
        assert_eq!(
            WorkabilityRating::default(),
            WorkabilityRating::NotApplicable
        );
        assert_eq!(
            NailCrackBehavior::default(),
            NailCrackBehavior::NotApplicable
        );
        assert_eq!(
            TreatabilityRating::default(),
            TreatabilityRating::NotApplicable
        );

        let error =
            serde_json::from_value::<TreatabilitiesSchema>(json!({"heartwood": "excellent"}));

        assert!(error.is_err());
    }
}
