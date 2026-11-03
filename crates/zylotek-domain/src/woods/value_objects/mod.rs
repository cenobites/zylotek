pub mod details;
mod gap_filling;
pub mod measurements;
pub mod mechanical_properties;
pub mod names;
pub mod physical_properties;
pub mod workabilities;

pub use details::{
    Attributes, KilnDrying, SourceReference, WebUrl, WoodDescription, WoodDetails, WoodImages,
};
pub use measurements::{
    Days, GramsPerCubicCentimeter, Kgf, KgfPerCentimeter, KgfPerSquareCentimeter, Measurement,
    Percentage,
};
pub use mechanical_properties::{
    Compression, JankaHardness, MechanicalProperties, MechanicalPropertiesByCondition,
    MoistureCondition, NailExtraction, StaticBending,
};
pub use names::{BotanicalFamily, CommonNames, ScientificName, WoodName};
pub use physical_properties::{Density, PhysicalProperties, Shrinkage};
pub use workabilities::{
    EquipmentFinishing, EquipmentFinishingResult, NailCrackBehavior, SurfaceFinishing,
    Treatabilities, TreatabilityRating, Workabilities, WorkabilityDefect,
    WorkabilityEquipmentSettings, WorkabilityRating,
};
