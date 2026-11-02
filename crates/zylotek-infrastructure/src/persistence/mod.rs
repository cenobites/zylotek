use crate::errors::InfrastructureError;

pub mod errors;
mod migrations;
pub mod woods;

pub use migrations::cli;

pub async fn build_db(database_uri: &str) -> Result<toasty::Db, InfrastructureError> {
    let db = toasty::Db::builder()
        .models(toasty::models!(crate::*))
        .connect(database_uri)
        .await?;

    db.push_schema().await?;

    Ok(db)
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
    async fn build_db_rejects_an_unknown_scheme() {
        let error = build_db("nosuchdb://localhost").await.unwrap_err();

        assert!(matches!(error, InfrastructureError::Database(_)));
    }
}
