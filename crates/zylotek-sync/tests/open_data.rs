//! The parser against the real LPF open-data export bundled in `toasty/fixtures`.

use std::{collections::HashSet, path::Path};

use zylotek_domain::woods::value_objects::Measurement;
use zylotek_sync::{DATASET_URL, OpenData, ScrapedWood, parse_open_data};

fn load() -> OpenData {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../toasty/fixtures/dados_abertos_banco_de_dados_de_madeiras_brasileiras_lpfsfb.csv",
    );

    parse_open_data(&std::fs::read(path).expect("the bundled CSV exists")).unwrap()
}

fn by_species_id(data: &OpenData, id: u64) -> &ScrapedWood {
    data.woods
        .iter()
        .find(|wood| wood.details.source.as_ref().unwrap().species_id == id)
        .unwrap_or_else(|| panic!("species {id} is in the file"))
}

#[test]
fn every_row_of_the_real_file_parses() {
    let data = load();

    assert_eq!(data.rejected, [], "no row is rejected");
    assert_eq!(data.woods.len(), 269);
}

#[test]
fn rows_are_test_lots_so_scientific_names_repeat() {
    let data = load();

    let names: HashSet<_> = data
        .woods
        .iter()
        .map(|w| w.scientific_name.as_str())
        .collect();
    let ids: HashSet<_> = data
        .woods
        .iter()
        .map(|w| w.details.source.as_ref().unwrap().species_id)
        .collect();

    assert_eq!(ids.len(), 269, "every row has its own id_especie");
    // 232 raw `nome_cientifico` values, but seven differ only by their "= synonym" suffix.
    assert_eq!(names.len(), 225, "but only 225 distinct accepted names");
}

#[test]
fn synonyms_are_split_off_the_accepted_name() {
    let data = load();

    let with_synonym: Vec<_> = data
        .woods
        .iter()
        .filter(|w| {
            w.details
                .description
                .general_characteristics
                .get("sinonímia")
                .is_some()
        })
        .collect();

    assert_eq!(with_synonym.len(), 52);
    assert!(
        data.woods
            .iter()
            .all(|w| !w.scientific_name.as_str().contains('='))
    );
    let ipe = by_species_id(&data, 245);
    assert_eq!(ipe.scientific_name.as_str(), "Handroanthus serratifolius");
    assert_eq!(
        ipe.details
            .description
            .general_characteristics
            .get("sinonímia"),
        Some("Tabebuia serratifolia")
    );
}

#[test]
fn a_known_row_is_converted_to_the_domains_units() {
    let data = load();

    let wood = by_species_id(&data, 21);
    let details = &wood.details;
    let green = &details.mechanical_properties.green;

    assert_eq!(wood.name.as_str(), "Muiracatiara-rajada");
    assert_eq!(wood.scientific_name.as_str(), "Astronium lecointei");
    assert_eq!(details.family.as_ref().unwrap().as_str(), "Anacardiaceae");
    assert_eq!(details.collection_location.as_deref(), Some("Belém, PA"));
    assert_eq!(
        details.source.as_ref().unwrap().page_url.as_str(),
        DATASET_URL
    );
    assert_eq!(
        details.physical_properties.density.basic.unwrap().value(),
        0.75
    );
    assert_eq!(
        green.static_bending.modulus_of_rupture.unwrap().value(),
        1026.04
    );
    assert_eq!(
        green.static_bending.modulus_of_elasticity.unwrap().value(),
        115_023.99
    );
    assert_eq!(
        green.janka_hardness.parallel_to_grain.unwrap().value(),
        672.0
    );
    assert_eq!(green.cleavage_strength, None, "blank in the file");
    assert_eq!(
        details
            .description
            .general_characteristics
            .get("autor do nome científico"),
        Some("Ducke")
    );
}

#[test]
fn thousands_separators_and_padded_cells_are_understood() {
    let data = load();

    // "1.059,12" N is 108 kgf.
    let nailed = by_species_id(&data, 31);
    assert_eq!(
        nailed
            .details
            .mechanical_properties
            .green
            .nail_extraction
            .parallel_to_grain
            .unwrap()
            .value(),
        108.0
    );
    for wood in &data.woods {
        let location = wood.details.collection_location.as_deref().unwrap();
        assert_eq!(location, location.trim(), "padded 'PA ' states are trimmed");
        assert!(
            ["PA", "AM", "RO", "MA"]
                .iter()
                .any(|state| location.ends_with(state)),
            "{location}"
        );
    }
}

#[test]
fn impossible_zeros_are_treated_as_not_measured() {
    let data = load();

    assert_eq!(
        by_species_id(&data, 63)
            .details
            .physical_properties
            .density
            .basic,
        None
    );
    assert_eq!(
        by_species_id(&data, 213)
            .details
            .mechanical_properties
            .green
            .cleavage_strength,
        None
    );
}

#[test]
fn converted_values_land_in_plausible_ranges() {
    let data = load();

    for wood in &data.woods {
        let details = &wood.details;
        let (green, dry) = (
            &details.mechanical_properties.green,
            &details.mechanical_properties.dry,
        );
        for moe in [
            green.static_bending.modulus_of_elasticity,
            dry.static_bending.modulus_of_elasticity,
        ]
        .into_iter()
        .flatten()
        {
            // 4 to 19 GPa: a MPa/GPa mix-up would be off by a factor of 1000.
            assert!(
                (40_000.0..200_000.0).contains(&moe.value()),
                "{} MOE {moe}",
                wood.scientific_name
            );
        }
        for mor in [
            green.static_bending.modulus_of_rupture,
            dry.static_bending.modulus_of_rupture,
        ]
        .into_iter()
        .flatten()
        {
            assert!(
                (200.0..2_000.0).contains(&mor.value()),
                "{} MOR {mor}",
                wood.scientific_name
            );
        }
        for janka in [
            green.janka_hardness.parallel_to_grain,
            dry.janka_hardness.parallel_to_grain,
        ]
        .into_iter()
        .flatten()
        {
            assert!(
                (100.0..2_500.0).contains(&janka.value()),
                "{} Janka {janka}",
                wood.scientific_name
            );
        }
        if let Some(density) = details.physical_properties.density.basic {
            assert!(
                (0.1..1.5).contains(&density.value()),
                "{} density {density}",
                wood.scientific_name
            );
        }
    }
}

#[test]
fn every_wood_has_a_name_family_and_source() {
    let data = load();

    for wood in &data.woods {
        let first = wood.name.as_str().chars().next().unwrap();
        assert!(first.is_uppercase(), "{}", wood.name);
        assert!(wood.details.family.is_some(), "{}", wood.scientific_name);
        assert!(wood.details.source.is_some());
    }
}
