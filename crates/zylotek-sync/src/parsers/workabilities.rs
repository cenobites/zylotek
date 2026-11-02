use std::collections::HashMap;

use anyhow::{Result, anyhow};
use scraper::ElementRef;
use zylotek_domain::woods::value_objects::{
    EquipmentFinishing, EquipmentFinishingResult, NailCrackBehavior, SurfaceFinishing,
    Treatabilities, TreatabilityRating, Workabilities, WorkabilityDefect,
    WorkabilityEquipmentSettings, WorkabilityRating,
};

use crate::parsers::{helpers, selectors};

pub(crate) fn parse(container: ElementRef<'_>) -> Result<Workabilities> {
    Ok(Workabilities {
        equipment_finishing: parse_table_equipment_finishing(container)?,
        surface_finishing: parse_table_type_of_action(container)?,
        treatabilities: parse_table_treatability_of_wood(container)?,
    })
}

fn parse_workability_defect(value: &str) -> Result<WorkabilityDefect> {
    match helpers::normalize(value).to_lowercase().as_str() {
        "-" | "" => Ok(WorkabilityDefect::NotApplicable),
        "arrancada, esmagada e felpuda" => Ok(WorkabilityDefect::CrushedAndFluffyAndRippedOff),
        "arrancada e esmagada" => Ok(WorkabilityDefect::CrushedAndRippedOff),
        "esmagada" => Ok(WorkabilityDefect::Crushed),
        "felpuda e arrancada" | "arrancada e felpuda" => Ok(WorkabilityDefect::FluffyAndRippedOff),
        "felpuda e riscada" | "riscada e felpuda" => Ok(WorkabilityDefect::FluffyAndScratched),
        "felpuda, arrancada e levantada" => Ok(WorkabilityDefect::FluffyAndRippedOffAndLifted),
        "felpuda esmagada levantada" => Ok(WorkabilityDefect::FluffyAndCrushedAndLifted),
        "felpuda e esmagada" => Ok(WorkabilityDefect::FluffyAndCrushed),
        "felpuda" => Ok(WorkabilityDefect::Fluffy),
        "arrancada" => Ok(WorkabilityDefect::RippedOff),
        "levantada e arrancada" => Ok(WorkabilityDefect::LiftedAndRippedOff),
        "levantada" => Ok(WorkabilityDefect::Lifted),
        "riscada" => Ok(WorkabilityDefect::Scratched),
        "aceitável" | "aceitavel" => Ok(WorkabilityDefect::Fair),
        "sem defeito" => Ok(WorkabilityDefect::Flawless),
        other => Err(anyhow!("unknown workability defect value: {other:?}")),
    }
}

fn parse_workability_equipment(value: &str) -> Result<WorkabilityEquipmentSettings> {
    match helpers::normalize(value).to_lowercase().as_str() {
        "-" | "" => Ok(WorkabilityEquipmentSettings::NotApplicable),
        "não adequada" => Ok(WorkabilityEquipmentSettings::NotSuitable),
        "adequada" => Ok(WorkabilityEquipmentSettings::Suitable),
        other => Err(anyhow!(
            "unknown workability equipment settings value: {other:?}"
        )),
    }
}

fn parse_workability_rating(value: &str) -> Result<WorkabilityRating> {
    match helpers::normalize(value).to_lowercase().as_str() {
        "-" | "" => Ok(WorkabilityRating::NotApplicable),
        "ruim" => Ok(WorkabilityRating::Poor),
        "muito ruim" => Ok(WorkabilityRating::VeryPoor),
        "regular" | "aceitável" => Ok(WorkabilityRating::Fair),
        "fácil" => Ok(WorkabilityRating::Easy),
        "bom" => Ok(WorkabilityRating::Good),
        "muito bom" => Ok(WorkabilityRating::VeryGood),
        "excelente" => Ok(WorkabilityRating::Excellent),
        other => Err(anyhow!("unknown workability rating value: {other:?}")),
    }
}

fn parse_nail_crack(value: &str) -> Result<NailCrackBehavior> {
    match helpers::normalize(value).to_lowercase().as_str() {
        "-" | "" => Ok(NailCrackBehavior::NotApplicable),
        "não trinca" => Ok(NailCrackBehavior::DoesNotCrack),
        "não trinca, exceto cantos" => Ok(NailCrackBehavior::DoesNotCrackExceptCorners),
        "trinca" => Ok(NailCrackBehavior::Cracks),
        other => Err(anyhow!("unknown nail-crack value: {other:?}")),
    }
}

fn parse_treatability(value: &str) -> Result<TreatabilityRating> {
    match helpers::normalize(value).to_lowercase().as_str() {
        "-" | "" => Ok(TreatabilityRating::NotApplicable),
        "verde" => Ok(TreatabilityRating::Green),
        "seca" => Ok(TreatabilityRating::Dry),
        "muito difícil" => Ok(TreatabilityRating::VeryHard),
        "moderadamente difícil" => Ok(TreatabilityRating::ModeratelyHard),
        "difícil" => Ok(TreatabilityRating::Hard),
        "fácil" => Ok(TreatabilityRating::Easy),
        "moderadamente fácil" => Ok(TreatabilityRating::ModeratelyEasy),
        "muito fácil" => Ok(TreatabilityRating::VeryEasy),
        other => Err(anyhow!("unknown treatability value: {other:?}")),
    }
}

fn parse_table_equipment_finishing(container: ElementRef<'_>) -> Result<EquipmentFinishing> {
    let table_selector = selectors::query("table")?;
    let rows_selector = selectors::query(r#"table > tbody > tr"#)?;
    let td_selector = selectors::query("td")?;

    let table_equipment_finishing_header_rows = [
        html::EQUIPMENT_FINISH_NUMBER_OF_SAMPLES_TABLE_HEADER_ROW,
        html::EQUIPMENT_FINISH_SURFICE_FINISHING_TABLE_HEADER_ROW,
        html::EQUIPMENT_FINISH_DEFECT_TABLE_HEADER_ROW,
        html::EQUIPMENT_FINISH_EQUIPMENT_SETTINGS_TABLE_HEADER_ROW,
    ];

    let mut table_rows = HashMap::new();
    let first_table = container.select(&table_selector).next().unwrap();
    for row_element in first_table.select(&rows_selector) {
        let mut cell_elements = row_element.select(&td_selector);
        if let Some(cell_element) = cell_elements.next() {
            let cell_text = cell_element
                .text()
                .collect::<String>()
                .trim()
                .to_lowercase();
            let attr_name = cell_text.clone();
            if table_equipment_finishing_header_rows.contains(&attr_name.as_str()) {
                let cells: Vec<String> = cell_elements.map(selectors::text).collect();
                table_rows.insert(attr_name, cells);
                continue;
            }
        }
    }

    let number_of_samples_row = table_rows
        .remove(html::EQUIPMENT_FINISH_NUMBER_OF_SAMPLES_TABLE_HEADER_ROW)
        .unwrap_or_default();

    let surface_finishing_row = table_rows
        .remove(html::EQUIPMENT_FINISH_SURFICE_FINISHING_TABLE_HEADER_ROW)
        .unwrap_or_default();

    let defect_row = table_rows
        .remove(html::EQUIPMENT_FINISH_DEFECT_TABLE_HEADER_ROW)
        .unwrap_or_default();

    let equipment_settings_row = table_rows
        .remove(html::EQUIPMENT_FINISH_EQUIPMENT_SETTINGS_TABLE_HEADER_ROW)
        .unwrap_or_default();

    // Columns, in order: molding planer, narrow belt sander, lathe, drill bit.
    let result = |column: usize| -> Result<EquipmentFinishingResult> {
        let cell = |row: &[String]| row.get(column).cloned().unwrap_or_default();

        Ok(EquipmentFinishingResult {
            number_of_samples: helpers::optional_number(number_of_samples_row.get(column))
                .and_then(|value| u32::try_from(value).ok()),
            surface_finish: parse_workability_rating(&cell(&surface_finishing_row))?,
            defect: parse_workability_defect(&cell(&defect_row))?,
            equipment_settings: parse_workability_equipment(&cell(&equipment_settings_row))?,
        })
    };

    Ok(EquipmentFinishing {
        molding_planer: result(0)?,
        narrow_belt_sander: result(1)?,
        lathe: result(2)?,
        drill_bit: result(3)?,
    })
}

fn parse_table_type_of_action(container: ElementRef<'_>) -> Result<SurfaceFinishing> {
    let table_rows =
        selectors::table_rows_by_heading(container, html::TYPE_OF_ACTION_TABLE_HEADING)
            .unwrap_or(vec![vec![String::new(); 6]; 2]);

    let surface_finishing_row = table_rows
        .get(1)
        .ok_or_else(|| anyhow!("workability - type of action surface finishing row missing"))?;

    Ok(SurfaceFinishing {
        planer: parse_workability_rating(&surface_finishing_row[1])?,
        sandpaper: parse_workability_rating(&surface_finishing_row[2])?,
        vise: parse_workability_rating(&surface_finishing_row[3])?,
        drill: parse_workability_rating(&surface_finishing_row[4])?,
        nail_crack: parse_nail_crack(&surface_finishing_row[5])?,
    })
}

fn parse_table_treatability_of_wood(container: ElementRef<'_>) -> Result<Treatabilities> {
    let table_rows =
        selectors::table_rows_by_heading(container, html::TREATABILITY_OF_WOOD_TABLE_HEADING)
            .unwrap_or(vec![vec![String::new(); 2]; 2]);

    let treatability_of_wood_row = table_rows
        .get(1)
        .ok_or_else(|| anyhow!("workability - treatability of wood row missing"))?;

    Ok(Treatabilities {
        heartwood: parse_treatability(&treatability_of_wood_row[0])?,
        sapwood: parse_treatability(&treatability_of_wood_row[1])?,
    })
}

mod html {
    // table equipment finish
    pub const EQUIPMENT_FINISH_NUMBER_OF_SAMPLES_TABLE_HEADER_ROW: &str = "número de amostras";
    pub const EQUIPMENT_FINISH_SURFICE_FINISHING_TABLE_HEADER_ROW: &str = "acabamento superficial";
    pub const EQUIPMENT_FINISH_DEFECT_TABLE_HEADER_ROW: &str = "defeito";
    pub const EQUIPMENT_FINISH_EQUIPMENT_SETTINGS_TABLE_HEADER_ROW: &str =
        "regulagem de equipamento";

    // table type of action
    pub const TYPE_OF_ACTION_TABLE_HEADING: &str = "trabalhabilidade - tipo de ação";

    // table treatability of wood
    pub const TREATABILITY_OF_WOOD_TABLE_HEADING: &str = "tratabilidade da madeira";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_map_from_portuguese() {
        let cases = [
            ("-", WorkabilityRating::NotApplicable),
            ("", WorkabilityRating::NotApplicable),
            ("Ruim", WorkabilityRating::Poor),
            ("muito  ruim", WorkabilityRating::VeryPoor),
            ("Regular", WorkabilityRating::Fair),
            ("aceitável", WorkabilityRating::Fair),
            ("fácil", WorkabilityRating::Easy),
            ("Bom", WorkabilityRating::Good),
            ("MUITO BOM", WorkabilityRating::VeryGood),
            ("excelente", WorkabilityRating::Excellent),
        ];

        for (input, expected) in cases {
            assert_eq!(
                parse_workability_rating(input).unwrap(),
                expected,
                "{input:?}"
            );
        }
        assert!(
            parse_workability_rating("péssimo")
                .unwrap_err()
                .to_string()
                .contains("péssimo")
        );
    }

    #[test]
    fn defects_map_from_portuguese() {
        let cases = [
            ("-", WorkabilityDefect::NotApplicable),
            (
                "arrancada, esmagada e felpuda",
                WorkabilityDefect::CrushedAndFluffyAndRippedOff,
            ),
            (
                "arrancada e esmagada",
                WorkabilityDefect::CrushedAndRippedOff,
            ),
            ("esmagada", WorkabilityDefect::Crushed),
            ("felpuda e arrancada", WorkabilityDefect::FluffyAndRippedOff),
            ("arrancada e felpuda", WorkabilityDefect::FluffyAndRippedOff),
            ("felpuda e riscada", WorkabilityDefect::FluffyAndScratched),
            ("riscada e felpuda", WorkabilityDefect::FluffyAndScratched),
            (
                "felpuda, arrancada e levantada",
                WorkabilityDefect::FluffyAndRippedOffAndLifted,
            ),
            (
                "felpuda esmagada levantada",
                WorkabilityDefect::FluffyAndCrushedAndLifted,
            ),
            ("felpuda e esmagada", WorkabilityDefect::FluffyAndCrushed),
            ("Felpuda", WorkabilityDefect::Fluffy),
            ("arrancada", WorkabilityDefect::RippedOff),
            (
                "levantada e arrancada",
                WorkabilityDefect::LiftedAndRippedOff,
            ),
            ("levantada", WorkabilityDefect::Lifted),
            ("riscada", WorkabilityDefect::Scratched),
            ("aceitável", WorkabilityDefect::Fair),
            ("aceitavel", WorkabilityDefect::Fair),
            ("sem defeito", WorkabilityDefect::Flawless),
        ];

        for (input, expected) in cases {
            assert_eq!(
                parse_workability_defect(input).unwrap(),
                expected,
                "{input:?}"
            );
        }
        assert!(parse_workability_defect("quebrada").is_err());
    }

    #[test]
    fn equipment_settings_nail_cracks_and_treatability_map_from_portuguese() {
        assert_eq!(
            parse_workability_equipment("").unwrap(),
            WorkabilityEquipmentSettings::NotApplicable
        );
        assert_eq!(
            parse_workability_equipment("não adequada").unwrap(),
            WorkabilityEquipmentSettings::NotSuitable
        );
        assert_eq!(
            parse_workability_equipment("Adequada").unwrap(),
            WorkabilityEquipmentSettings::Suitable
        );
        assert!(parse_workability_equipment("meio").is_err());

        assert_eq!(
            parse_nail_crack("-").unwrap(),
            NailCrackBehavior::NotApplicable
        );
        assert_eq!(
            parse_nail_crack("não trinca").unwrap(),
            NailCrackBehavior::DoesNotCrack
        );
        assert_eq!(
            parse_nail_crack("Trinca").unwrap(),
            NailCrackBehavior::Cracks
        );
        assert!(parse_nail_crack("rachou").is_err());

        let treatabilities = [
            ("-", TreatabilityRating::NotApplicable),
            ("verde", TreatabilityRating::Green),
            ("seca", TreatabilityRating::Dry),
            ("muito difícil", TreatabilityRating::VeryHard),
            ("moderadamente difícil", TreatabilityRating::ModeratelyHard),
            ("difícil", TreatabilityRating::Hard),
            ("fácil", TreatabilityRating::Easy),
            ("moderadamente fácil", TreatabilityRating::ModeratelyEasy),
            ("muito fácil", TreatabilityRating::VeryEasy),
        ];
        for (input, expected) in treatabilities {
            assert_eq!(parse_treatability(input).unwrap(), expected, "{input:?}");
        }
        assert!(parse_treatability("impossível").is_err());
    }
}
