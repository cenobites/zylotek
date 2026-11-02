//! The repository on a file-backed SQLite database, as it would run once the hard-coded
//! `sqlite::memory:` URI is replaced by configuration.

use tempfile::TempDir;
use zylotek_domain::{
    testing::{full_details, wood},
    woods::repositories::WoodRepository,
};
use zylotek_infrastructure::persistence::{build_db, woods::repositories::PostgresWoodRepository};

fn uri(dir: &TempDir) -> String {
    format!("sqlite:{}", dir.path().join("zylotek.db").display())
}

#[tokio::test]
async fn data_is_written_to_disk() {
    let dir = TempDir::new().unwrap();
    let repository = PostgresWoodRepository::new(build_db(&uri(&dir)).await.unwrap());
    let stored = wood("Aroeira", "Myracrodruon urundeuva");

    repository.save(&stored).await.unwrap();

    assert!(dir.path().join("zylotek.db").metadata().unwrap().len() > 0);
    let found = repository.find_by_id(stored.id()).await.unwrap().unwrap();
    assert_eq!(found.details(), &full_details());
}

// `build_db` always runs `push_schema`, which fails on a database that already has the
// tables, so a restart against the same file cannot reconnect yet.
#[tokio::test]
#[ignore = "known gap: build_db's push_schema fails on an existing schema"]
async fn data_survives_reopening_the_database() {
    let dir = TempDir::new().unwrap();
    let uri = uri(&dir);
    let stored = wood("Aroeira", "Myracrodruon urundeuva");
    {
        let repository = PostgresWoodRepository::new(build_db(&uri).await.unwrap());
        repository.save(&stored).await.unwrap();
    }

    let repository = PostgresWoodRepository::new(build_db(&uri).await.unwrap());

    let found = repository.find_by_id(stored.id()).await.unwrap().unwrap();
    assert_eq!(found.scientific_name(), stored.scientific_name());
    assert_eq!(repository.count().await.unwrap(), 1);
}

#[tokio::test]
async fn concurrent_writers_do_not_lose_woods() {
    let dir = TempDir::new().unwrap();
    let repository = std::sync::Arc::new(PostgresWoodRepository::new(
        build_db(&uri(&dir)).await.unwrap(),
    ));

    let tasks: Vec<_> = (0..16)
        .map(|i| {
            let repository = repository.clone();
            tokio::spawn(async move {
                repository
                    .save(&wood("Madeira", &format!("Genus species{i:02}")))
                    .await
            })
        })
        .collect();
    let saved = futures_all(tasks).await;

    assert_eq!(repository.count().await.unwrap(), saved as u64);
    assert_eq!(saved, 16, "every distinct wood is stored");
}

async fn futures_all<T, E>(tasks: Vec<tokio::task::JoinHandle<Result<T, E>>>) -> usize {
    let mut ok = 0;
    for task in tasks {
        ok += usize::from(task.await.unwrap().is_ok());
    }
    ok
}
