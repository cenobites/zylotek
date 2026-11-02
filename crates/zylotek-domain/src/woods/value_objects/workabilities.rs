#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workabilities {
    pub equipment_finishing: EquipmentFinishing,
    pub surface_finishing: SurfaceFinishing,
    pub treatabilities: Treatabilities,
}

/// Machining results per equipment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EquipmentFinishing {
    pub molding_planer: EquipmentFinishingResult,
    pub narrow_belt_sander: EquipmentFinishingResult,
    pub lathe: EquipmentFinishingResult,
    pub drill_bit: EquipmentFinishingResult,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EquipmentFinishingResult {
    pub number_of_samples: Option<u32>,
    pub surface_finish: WorkabilityRating,
    pub defect: WorkabilityDefect,
    pub equipment_settings: WorkabilityEquipmentSettings,
}

/// Surface finishing by type of action.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SurfaceFinishing {
    pub planer: WorkabilityRating,
    pub sandpaper: WorkabilityRating,
    pub vise: WorkabilityRating,
    pub drill: WorkabilityRating,
    pub nail_crack: NailCrackBehavior,
}

/// How easily preservatives penetrate the wood.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Treatabilities {
    pub heartwood: TreatabilityRating,
    pub sapwood: TreatabilityRating,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum WorkabilityRating {
    #[default]
    NotApplicable,
    VeryPoor,
    Poor,
    Fair,
    Easy,
    Good,
    VeryGood,
    Excellent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum WorkabilityDefect {
    #[default]
    NotApplicable,
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
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum WorkabilityEquipmentSettings {
    #[default]
    NotApplicable,
    NotSuitable,
    Suitable,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum NailCrackBehavior {
    #[default]
    NotApplicable,
    DoesNotCrack,
    DoesNotCrackExceptCorners,
    Cracks,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TreatabilityRating {
    #[default]
    NotApplicable,
    Green,
    Dry,
    VeryHard,
    Hard,
    ModeratelyHard,
    ModeratelyEasy,
    Easy,
    VeryEasy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_default_to_not_applicable() {
        let workabilities = Workabilities::default();

        assert_eq!(
            workabilities.surface_finishing.planer,
            WorkabilityRating::NotApplicable
        );
        assert_eq!(
            workabilities.surface_finishing.nail_crack,
            NailCrackBehavior::NotApplicable
        );
        assert_eq!(
            workabilities.treatabilities.heartwood,
            TreatabilityRating::NotApplicable
        );

        let result = EquipmentFinishingResult::default();
        assert_eq!(result.number_of_samples, None);
        assert_eq!(result.defect, WorkabilityDefect::NotApplicable);
        assert_eq!(
            result.equipment_settings,
            WorkabilityEquipmentSettings::NotApplicable
        );
        assert_eq!(
            workabilities.equipment_finishing,
            EquipmentFinishing::default()
        );
        assert_eq!(workabilities.surface_finishing, SurfaceFinishing::default());
        assert_eq!(workabilities.treatabilities, Treatabilities::default());
    }

    #[test]
    fn workabilities_are_cloneable_and_debuggable() {
        let workabilities = Workabilities::default();

        assert_eq!(workabilities.clone(), workabilities);
        assert!(format!("{workabilities:?}").contains("NotApplicable"));
    }
}
