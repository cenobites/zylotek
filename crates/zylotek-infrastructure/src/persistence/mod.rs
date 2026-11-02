use crate::errors::InfrastructureError;

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
    let db = toasty::Db::builder()
        .models(toasty::models!(crate::*))
        .connect(database_uri)
        .await?;

    if !has_schema(&db).await {
        db.push_schema().await?;
    }

    Ok(db)
}

/// `push_schema` fails on tables that already exist, so probe for them first.
async fn has_schema(db: &toasty::Db) -> bool {
    let mut db = db.clone();

    woods::models::Wood::all()
        .count()
        .exec(&mut db)
        .await
        .is_ok()
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
