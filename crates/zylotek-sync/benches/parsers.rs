//! Parsing a scraped species page: the CPU-bound half of the ingest flow.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use zylotek_sync::parse_wood;

const SPECIES_HTML: &str = include_str!("../tests/fixtures/species.html");
const URL: &str =
    "https://lpf.florestal.gov.br/pt-br/madeiras-brasileiras/especie?especieestudadaid=42";

fn parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_wood");
    group.throughput(Throughput::Bytes(SPECIES_HTML.len() as u64));

    group.bench_function("species_page", |b| {
        b.iter(|| parse_wood(black_box(SPECIES_HTML), black_box(URL)).unwrap())
    });

    // Pages from a full sync (~270 species) are parsed back to back.
    group.bench_function("failure_fast", |b| {
        b.iter(|| {
            parse_wood(black_box("<html><body>nada</body></html>"), black_box(URL)).unwrap_err()
        })
    });

    let large = SPECIES_HTML.replace(
        "</div>\n</body>",
        &format!("{}</div>\n</body>", "<p>ruído</p>".repeat(2_000)),
    );
    group.throughput(Throughput::Bytes(large.len() as u64));
    group.bench_function("species_page_with_2000_extra_nodes", |b| {
        b.iter(|| parse_wood(black_box(&large), black_box(URL)).unwrap())
    });
    group.finish();
}

criterion_group!(benches, parse);
criterion_main!(benches);
