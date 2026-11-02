//! Ranking the whole catalogue (225 woods, the size of the open-data set) for the question
//! shape the assistant sees most: four weighted criteria.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use zylotek_assistant::{ranking::Ranker, requirements::Requirements};
use zylotek_contracts::{
    assistant::Property,
    woods::{WoodDetailsSchema, WoodResponse, WorkabilityRating},
};

const WOODS: usize = 225;

fn catalogue() -> Vec<WoodResponse> {
    (0..WOODS)
        .map(|n| {
            let spread = (n * 7919 % 1000) as f64 / 1000.0;
            let mut details = WoodDetailsSchema::default();
            details
                .mechanical_properties
                .dry
                .janka_hardness
                .parallel_to_grain = Some(200.0 + spread * 3000.0);
            details
                .mechanical_properties
                .dry
                .static_bending
                .modulus_of_rupture = Some(500.0 + spread * 1500.0);
            details
                .mechanical_properties
                .dry
                .static_bending
                .modulus_of_elasticity = Some(70_000.0 + spread * 150_000.0);
            details.physical_properties.shrinkage.volumetric = Some(5.0 + spread * 15.0);
            details.physical_properties.density.apparent = Some(0.3 + spread);
            details.workabilities.surface_finishing.planer = WorkabilityRating::Good;
            details.workabilities.surface_finishing.sandpaper = WorkabilityRating::Easy;

            WoodResponse {
                id: format!("id-{n}"),
                name: format!("Wood {n}"),
                scientific_name: format!("Genus species{n}"),
                genus: "Genus".into(),
                details,
            }
        })
        .collect()
}

fn requirements() -> Requirements {
    Requirements::parse(
        r#"{"summary": "a large table", "priorities": [
            {"property": "stability", "weight": 3}, {"property": "workability", "weight": 3},
            {"property": "stiffness", "weight": 2}, {"property": "hardness", "weight": 2}],
            "avoid": ["wood 7"]}"#,
    )
    .unwrap()
}

fn benches(c: &mut Criterion) {
    let woods = catalogue();
    let ranker = Ranker::new(woods.clone());
    let requirements = requirements();
    assert_eq!(ranker.rank(&requirements, 5).len(), 5);
    assert!(Property::ALL.len() > 4);

    c.bench_function("ranker/build_225_woods", |b| {
        b.iter(|| Ranker::new(black_box(woods.clone())))
    });
    c.bench_function("ranker/rank_top_5", |b| {
        b.iter(|| ranker.rank(black_box(&requirements), 5))
    });
    c.bench_function("requirements/parse_model_reply", |b| {
        let reply = r#"```json
        {"summary": "a large table", "priorities": [{"property": "stability", "weight": 3},
        {"property": "workability", "weight": 3}], "avoid": []}
        ```"#;
        b.iter(|| Requirements::parse(black_box(reply)).unwrap())
    });
}

criterion_group!(group, benches);
criterion_main!(group);
