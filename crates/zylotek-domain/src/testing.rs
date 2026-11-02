//! Test support: fixtures and an in-memory [`WoodRepository`].
//!
//! Compiled for this crate's tests and, behind the `testing` feature, for other crates'
//! tests and benches. Never enable the feature in a production dependency.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use crate::{
    errors::RepositoryError,
    woods::{
        entities::{Wood, WoodId},
        repositories::WoodRepository,
        value_objects::*,
    },
};

/// A valid wood with every detail populated, so round-trip tests exercise every field.
pub fn full_details() -> WoodDetails {
    let url = |path: &str| WebUrl::new(format!("https://lpf.florestal.gov.br/{path}")).unwrap();
    let equipment =
        |samples, surface_finish, defect, equipment_settings| EquipmentFinishingResult {
            number_of_samples: Some(samples),
            surface_finish,
            defect,
            equipment_settings,
        };
    let mechanical = |scale: f64| MechanicalPropertiesByCondition {
        static_bending: StaticBending {
            modulus_of_rupture: Some(KgfPerSquareCentimeter::new(1_000.0 * scale).unwrap()),
            modulus_of_elasticity: Some(KgfPerSquareCentimeter::new(120_000.0 * scale).unwrap()),
        },
        compression: Compression {
            parallel_to_grain: Some(KgfPerSquareCentimeter::new(500.0 * scale).unwrap()),
            perpendicular_to_grain: Some(KgfPerSquareCentimeter::new(90.0 * scale).unwrap()),
        },
        janka_hardness: JankaHardness {
            parallel_to_grain: Some(Kgf::new(700.0 * scale).unwrap()),
            end_grain: Some(Kgf::new(800.0 * scale).unwrap()),
        },
        tension_perpendicular_to_grain: Some(KgfPerSquareCentimeter::new(60.0 * scale).unwrap()),
        cleavage_strength: Some(KgfPerCentimeter::new(70.0 * scale).unwrap()),
        shear_parallel_to_grain: Some(KgfPerSquareCentimeter::new(110.0 * scale).unwrap()),
        nail_extraction: NailExtraction {
            parallel_to_grain: Some(Kgf::new(150.0 * scale).unwrap()),
            end_grain: Some(Kgf::new(120.0 * scale).unwrap()),
        },
    };

    WoodDetails {
        family: Some(BotanicalFamily::new("Anacardiaceae").unwrap()),
        collection_location: Some("Pará".to_owned()),
        common_names: CommonNames::new(["Aroeira", "Gonçalo-alves"]),
        source: Some(SourceReference {
            species_id: 42,
            page_url: url("pt-br/component/madeirasbrasileiras/?especieestudadaid=42"),
        }),
        description: WoodDescription {
            tree: Attributes::new([("altura", "30 m"), ("diâmetro", "80 cm")]),
            general_characteristics: Attributes::new([("cor", "castanho"), ("grã", "irregular")]),
            end_uses: Some("Construção civil".to_owned()),
            preservation: Some("Alta durabilidade natural".to_owned()),
            workability_notes: Some("Fácil de trabalhar".to_owned()),
            heartwood_color_classification: Some("castanho".to_owned()),
        },
        kiln_drying: KilnDrying {
            schedule: Attributes::new([("temperatura inicial", "40 °C")]),
            duration: Some(Days::new(12.0).unwrap()),
        },
        images: WoodImages {
            main: Some(url("images/main.jpg")),
            tree: Some(url("images/tree.jpg")),
            log: Some(url("images/log.jpg")),
            bark: Some(url("images/bark.jpg")),
            tangential_cut: Some(url("images/tangential.jpg")),
            cross_section: Some(url("images/cross.jpg")),
            radial_cut: Some(url("images/radial.jpg")),
            wood: Some(url("images/wood.jpg")),
        },
        physical_properties: PhysicalProperties {
            density: Density {
                dry: Some(GramsPerCubicCentimeter::new(0.9).unwrap()),
                green: Some(GramsPerCubicCentimeter::new(1.1).unwrap()),
                basic: Some(GramsPerCubicCentimeter::new(0.8).unwrap()),
                apparent: Some(GramsPerCubicCentimeter::new(0.95).unwrap()),
            },
            shrinkage: Shrinkage {
                tangential: Some(Percentage::new(7.5).unwrap()),
                radial: Some(Percentage::new(5.0).unwrap()),
                volumetric: Some(Percentage::new(12.5).unwrap()),
            },
        },
        mechanical_properties: MechanicalProperties {
            green: mechanical(1.0),
            dry: mechanical(1.25),
        },
        workabilities: Workabilities {
            equipment_finishing: EquipmentFinishing {
                molding_planer: equipment(
                    10,
                    WorkabilityRating::Good,
                    WorkabilityDefect::Flawless,
                    WorkabilityEquipmentSettings::Suitable,
                ),
                narrow_belt_sander: equipment(
                    11,
                    WorkabilityRating::Excellent,
                    WorkabilityDefect::Fluffy,
                    WorkabilityEquipmentSettings::NotSuitable,
                ),
                lathe: equipment(
                    12,
                    WorkabilityRating::Fair,
                    WorkabilityDefect::CrushedAndRippedOff,
                    WorkabilityEquipmentSettings::Suitable,
                ),
                drill_bit: equipment(
                    13,
                    WorkabilityRating::VeryPoor,
                    WorkabilityDefect::LiftedAndRippedOff,
                    WorkabilityEquipmentSettings::NotApplicable,
                ),
            },
            surface_finishing: SurfaceFinishing {
                planer: WorkabilityRating::VeryGood,
                sandpaper: WorkabilityRating::Easy,
                vise: WorkabilityRating::Poor,
                drill: WorkabilityRating::Good,
                nail_crack: NailCrackBehavior::DoesNotCrackExceptCorners,
            },
            treatabilities: Treatabilities {
                heartwood: TreatabilityRating::ModeratelyHard,
                sapwood: TreatabilityRating::VeryEasy,
            },
        },
    }
}

/// A freshly created wood; its creation event is discarded.
pub fn wood(name: &str, scientific_name: &str) -> Wood {
    Wood::create(
        WoodName::new(name).unwrap(),
        ScientificName::new(scientific_name).unwrap(),
        full_details(),
    )
    .0
}

/// A [`WoodRepository`] operation, used to make [`InMemoryWoodRepository`] fail on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    ExistsByScientificName,
    FindById,
    List,
    Count,
    Save,
    Update,
    Delete,
}

/// A [`WoodRepository`] backed by a map, with per-operation fault injection.
#[derive(Default)]
pub struct InMemoryWoodRepository {
    woods: Mutex<HashMap<WoodId, Wood>>,
    failures: Mutex<HashSet<Operation>>,
}

impl InMemoryWoodRepository {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_woods(woods: impl IntoIterator<Item = Wood>) -> Self {
        let repository = Self::new();
        repository
            .woods
            .lock()
            .unwrap()
            .extend(woods.into_iter().map(|wood| (wood.id(), wood)));
        repository
    }

    /// Makes `operation` return [`RepositoryError::Unavailable`] from now on.
    pub fn fail_on(self, operation: Operation) -> Self {
        self.failures.lock().unwrap().insert(operation);
        self
    }

    pub fn len(&self) -> usize {
        self.woods.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn check(&self, operation: Operation) -> Result<(), RepositoryError> {
        if self.failures.lock().unwrap().contains(&operation) {
            return Err(RepositoryError::Unavailable);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl WoodRepository for InMemoryWoodRepository {
    async fn exists_by_scientific_name(
        &self,
        scientific_name: &str,
    ) -> Result<bool, RepositoryError> {
        self.check(Operation::ExistsByScientificName)?;
        Ok(self
            .woods
            .lock()
            .unwrap()
            .values()
            .any(|wood| wood.scientific_name().as_str() == scientific_name))
    }

    async fn find_by_id(&self, id: WoodId) -> Result<Option<Wood>, RepositoryError> {
        self.check(Operation::FindById)?;
        Ok(self.woods.lock().unwrap().get(&id).cloned())
    }

    async fn list(&self, offset: usize, limit: usize) -> Result<Vec<Wood>, RepositoryError> {
        self.check(Operation::List)?;
        let mut woods: Vec<Wood> = self.woods.lock().unwrap().values().cloned().collect();
        woods.sort_by(|a, b| {
            a.scientific_name()
                .as_str()
                .cmp(b.scientific_name().as_str())
        });
        Ok(woods.into_iter().skip(offset).take(limit).collect())
    }

    async fn count(&self) -> Result<u64, RepositoryError> {
        self.check(Operation::Count)?;
        Ok(self.len() as u64)
    }

    async fn save(&self, wood: &Wood) -> Result<Wood, RepositoryError> {
        self.check(Operation::Save)?;
        self.woods.lock().unwrap().insert(wood.id(), wood.clone());
        Ok(wood.clone())
    }

    async fn update(&self, wood: &Wood) -> Result<Wood, RepositoryError> {
        self.check(Operation::Update)?;
        let mut woods = self.woods.lock().unwrap();
        let stored = woods
            .get_mut(&wood.id())
            .ok_or(RepositoryError::OperationFailed)?;
        *stored = wood.clone();
        Ok(wood.clone())
    }

    async fn delete(&self, id: WoodId) -> Result<(), RepositoryError> {
        self.check(Operation::Delete)?;
        self.woods.lock().unwrap().remove(&id);
        Ok(())
    }
}
