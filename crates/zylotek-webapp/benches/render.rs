//! The per-request work the website does outside the network: reading the compare query and
//! turning a full wood into display strings.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use zylotek_contracts::{
    assistant::{Property, Reason, Recommendation},
    woods::{WoodDetailsSchema, WoodResponse, WorkabilityRating},
};
use zylotek_webapp::{i18n::Lang, pages::compare_ids, view};

fn wood() -> WoodResponse {
    let mut details = WoodDetailsSchema {
        family: Some("Meliaceae".into()),
        common_names: vec!["Cedro".into(), "Cedro-rosa".into()],
        ..Default::default()
    };
    details.physical_properties.density.apparent = Some(0.55);
    details.physical_properties.shrinkage.volumetric = Some(11.2);
    details
        .mechanical_properties
        .dry
        .janka_hardness
        .parallel_to_grain = Some(640.0);
    details
        .mechanical_properties
        .dry
        .static_bending
        .modulus_of_rupture = Some(870.0);
    details
        .mechanical_properties
        .dry
        .static_bending
        .modulus_of_elasticity = Some(98_000.0);
    details.workabilities.surface_finishing.planer = WorkabilityRating::Good;
    details.description.end_uses = Some("Furniture, joinery, boats".into());
    for n in 0..12 {
        details.description.general_characteristics.insert(
            format!("Trait {n}"),
            "Straight grain, medium texture".into(),
        );
    }

    WoodResponse {
        id: "0b9c4f1e-aaaa-4bbb-8ccc-123456789abc".into(),
        name: "Cedro".into(),
        scientific_name: "Cedrela fissilis".into(),
        genus: "Cedrela".into(),
        details,
    }
}

fn benches(c: &mut Criterion) {
    let wood = wood();
    let recommendation = Recommendation {
        wood_id: wood.id.clone(),
        name: wood.name.clone(),
        scientific_name: wood.scientific_name.clone(),
        score: 0.87,
        reasons: Property::ALL
            .into_iter()
            .map(|property| Reason {
                property,
                value: 1.0,
                fit: 0.5,
            })
            .collect(),
    };
    let query = "id=0b9c4f1e-aaaa-4bbb-8ccc-123456789abc&id=1b9c4f1e-aaaa-4bbb-8ccc-123456789abc&id=2b9c4f1e-aaaa-4bbb-8ccc-123456789abc";
    assert_eq!(compare_ids(Some(query)).unwrap().len(), 3);

    c.bench_function("compare_ids/three_woods", |b| {
        b.iter(|| compare_ids(black_box(Some(query))))
    });
    c.bench_function("view/facts", |b| {
        b.iter(|| view::facts(black_box(&wood), Lang::PtBr))
    });
    c.bench_function("view/detail", |b| {
        b.iter(|| view::detail(black_box(&wood), Lang::PtBr))
    });
    c.bench_function("view/card", |b| {
        b.iter(|| view::card(black_box(&recommendation), Lang::PtBr))
    });
}

criterion_group!(group, benches);
criterion_main!(group);
