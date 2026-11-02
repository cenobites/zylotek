//! Value-object construction: every request, stored row and scraped page goes through it.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use zylotek_domain::woods::{entities::Wood, value_objects::*};

fn names(c: &mut Criterion) {
    let mut group = c.benchmark_group("names");
    group.bench_function("wood_name", |b| {
        b.iter(|| WoodName::new(black_box("  Muiracatiara   rajada ")))
    });
    group.bench_function("scientific_name", |b| {
        b.iter(|| ScientificName::new(black_box("Astronium   lecointei")))
    });
    let common = [
        "Aroeira",
        "aroeira",
        "Gonçalo-alves",
        "Muiracatiara",
        "Maracatiara",
        "Aroeira-do-sertão",
        "Urundeúva",
        "AROEIRA",
        "Aroeira preta",
        "Aroeira-verdadeira",
    ];
    group.bench_function("common_names_10", |b| {
        b.iter(|| CommonNames::new(black_box(common)))
    });
    group.finish();
}

fn details(c: &mut Criterion) {
    let mut group = c.benchmark_group("details");
    group.bench_function("web_url", |b| {
        b.iter(|| WebUrl::new(black_box("https://lpf.florestal.gov.br/images/main.jpg")))
    });
    let pairs = [
        ("altura", " 30 m "),
        ("diâmetro", "80 cm"),
        ("copa", "ampla"),
        ("", "x"),
    ];
    group.bench_function("attributes_4", |b| {
        b.iter(|| Attributes::new(black_box(pairs)))
    });
    group.bench_function("measurement", |b| {
        b.iter(|| Percentage::new(black_box(42.0)))
    });
    group.finish();
}

fn aggregate(c: &mut Criterion) {
    let name = WoodName::new("Muiracatiara").unwrap();
    let scientific_name = ScientificName::new("Astronium lecointei").unwrap();
    let details = WoodDetails {
        family: Some(BotanicalFamily::new("Anacardiaceae").unwrap()),
        common_names: CommonNames::new(["Aroeira", "Gonçalo-alves"]),
        description: WoodDescription {
            tree: Attributes::new([("altura", "30 m"), ("diâmetro", "80 cm")]),
            ..Default::default()
        },
        ..Default::default()
    };

    c.bench_function("wood_create", |b| {
        b.iter(|| Wood::create(name.clone(), scientific_name.clone(), details.clone()))
    });
}

criterion_group!(benches, names, details, aggregate);
criterion_main!(benches);
