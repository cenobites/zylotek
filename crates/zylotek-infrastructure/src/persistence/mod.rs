use crate::errors::InfrastructureError;

pub mod asks;
pub mod errors;
mod migrations;
pub mod woods;

pub use migrations::cli;

/// Where `sync` and `api` keep their data unless told otherwise. Relative to the working
/// directory; use `sqlite::memory:` for a throwaway database.
pub const DEFAULT_DATABASE_URL: &str = "sqlite:zylotek.db";

/// Connects to `database_uri` and makes sure the tables exist, creating them on a new
/// database and leaving the data of an existing one untouched.
pub async fn build_db(database_uri: &str) -> Result<toasty::Db, InfrastructureError> {
    let db = connect(database_uri).await?;

    if !has_woods(&db).await {
        db.push_schema().await?;
    } else if !has_asks(&db).await {
        // A database made before asks existed: add their table and leave the rest alone.
        toasty::Db::builder()
            .models(toasty::models!(asks::models::Ask))
            .connect(database_uri)
            .await?
            .push_schema()
            .await?;
    }

    Ok(db)
}

/// Connects without touching the schema; the migration commands own it there.
pub(crate) async fn connect(database_uri: &str) -> Result<toasty::Db, InfrastructureError> {
    log::info!("Building database connection to {}", database_uri);
    Ok(toasty::Db::builder()
        .models(toasty::models!(crate::*))
        .connect(database_uri)
        .await?)
}

/// `push_schema` fails on tables that already exist, so probe for them first.
async fn has_woods(db: &toasty::Db) -> bool {
    let mut db = db.clone();

    woods::models::Wood::all()
        .count()
        .exec(&mut db)
        .await
        .is_ok()
}

async fn has_asks(db: &toasty::Db) -> bool {
    let mut db = db.clone();

    asks::models::Ask::all().count().exec(&mut db).await.is_ok()
}

impl From<toasty::Error> for InfrastructureError {
    fn from(err: toasty::Error) -> Self {
        Self::Database(err.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_db_connects_and_pushes_the_schema() {
        let db = build_db("sqlite::memory:").await.unwrap();

        // The schema exists, so counting rows works.
        let mut db = db;
        let count = woods::models::Wood::all()
            .count()
            .exec(&mut db)
            .await
            .unwrap();

        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn build_db_reopens_an_existing_database_and_keeps_its_data() {
        let dir = tempfile::tempdir().unwrap();
        let uri = format!("sqlite:{}", dir.path().join("woods.db").display());
        let seed = zylotek_domain::testing::wood("Aroeira", "Myracrodruon urundeuva");
        {
            use zylotek_domain::woods::repositories::WoodRepository;
            let repository =
                woods::repositories::PostgresWoodRepository::new(build_db(&uri).await.unwrap());
            repository.save(&seed).await.unwrap();
        }

        let mut reopened = build_db(&uri).await.unwrap();

        let count = woods::models::Wood::all()
            .count()
            .exec(&mut reopened)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn a_database_from_before_asks_gets_their_table_and_keeps_its_woods() {
        let dir = tempfile::tempdir().unwrap();
        let uri = format!("sqlite:{}", dir.path().join("old.db").display());
        // Only the woods table, as every database made before asks existed.
        let old = toasty::Db::builder()
            .models(toasty::models!(woods::models::Wood))
            .connect(&uri)
            .await
            .unwrap();
        old.push_schema().await.unwrap();
        {
            use zylotek_domain::woods::repositories::WoodRepository;
            woods::repositories::PostgresWoodRepository::new(old)
                .save(&zylotek_domain::testing::wood(
                    "Aroeira",
                    "Myracrodruon urundeuva",
                ))
                .await
                .unwrap();
        }

        let upgraded = build_db(&uri).await.unwrap();

        assert!(has_asks(&upgraded).await, "the asks table was added");
        let mut db = upgraded.clone();
        assert_eq!(
            woods::models::Wood::all()
                .count()
                .exec(&mut db)
                .await
                .unwrap(),
            1
        );
        let again = build_db(&uri).await.unwrap();
        assert!(
            has_woods(&again).await && has_asks(&again).await,
            "reopening changes nothing"
        );
    }

    #[test]
    fn the_default_database_is_a_file() {
        assert_eq!(DEFAULT_DATABASE_URL, "sqlite:zylotek.db");
    }

    #[tokio::test]
    async fn build_db_rejects_an_unknown_scheme() {
        let error = build_db("nosuchdb://localhost").await.unwrap_err();

        assert!(matches!(error, InfrastructureError::Database(_)));
    }
}
