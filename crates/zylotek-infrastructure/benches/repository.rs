//! Repository operations against an in-memory SQLite database seeded with 200 woods.

use std::{
    hint::black_box,
    sync::atomic::{AtomicU64, Ordering},
};

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use tokio::runtime::Runtime;
use zylotek_domain::{
    testing::wood,
    woods::{entities::WoodId, repositories::WoodRepository},
};
use zylotek_infrastructure::{
    persistence::woods::repositories::PostgresWoodRepository, testing::memory_db,
};

const SEEDED: usize = 200;

fn seeded(runtime: &Runtime) -> (PostgresWoodRepository, Vec<WoodId>) {
    runtime.block_on(async {
        let repository = PostgresWoodRepository::new(memory_db().await.unwrap());
        let mut ids = Vec::with_capacity(SEEDED);
        for i in 0..SEEDED {
            let seed = wood("Madeira", &format!("Genus species{i:04}"));
            repository.save(&seed).await.unwrap();
            ids.push(seed.id());
        }
        (repository, ids)
    })
}

fn reads(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let (repository, ids) = seeded(&runtime);
    let mut group = c.benchmark_group("repository/read");

    group.bench_function("find_by_id", |b| {
        b.to_async(&runtime).iter(|| async {
            repository
                .find_by_id(black_box(ids[SEEDED / 2]))
                .await
                .unwrap()
        })
    });
    group.bench_function("find_by_id_missing", |b| {
        b.to_async(&runtime).iter(|| async {
            repository
                .find_by_id(black_box(WoodId::new()))
                .await
                .unwrap()
        })
    });
    group.bench_function("exists_by_scientific_name", |b| {
        b.to_async(&runtime).iter(|| async {
            repository
                .exists_by_scientific_name(black_box("Genus species0100"))
                .await
                .unwrap()
        })
    });
    for limit in [20, 100] {
        group.bench_function(format!("list_{limit}"), |b| {
            b.to_async(&runtime).iter(|| async {
                repository
                    .list(black_box(50), black_box(limit))
                    .await
                    .unwrap()
            })
        });
    }
    group.bench_function("count", |b| {
        b.to_async(&runtime)
            .iter(|| async { repository.count().await.unwrap() })
    });
    group.finish();
}

fn writes(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let (repository, ids) = seeded(&runtime);
    let counter = AtomicU64::new(0);
    let mut group = c.benchmark_group("repository/write");

    group.bench_function("save_then_delete", |b| {
        let repository = &repository;
        b.to_async(&runtime).iter_batched(
            || {
                wood(
                    "Madeira",
                    &format!("Bench item{}", counter.fetch_add(1, Ordering::Relaxed)),
                )
            },
            |new| async move {
                repository.save(&new).await.unwrap();
                repository.delete(new.id()).await.unwrap();
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("update", |b| {
        let existing = runtime
            .block_on(repository.find_by_id(ids[0]))
            .unwrap()
            .unwrap();
        b.to_async(&runtime)
            .iter(|| async { repository.update(black_box(&existing)).await.unwrap() })
    });
    group.finish();
}

criterion_group!(benches, reads, writes);
criterion_main!(benches);
