#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("database error")]
    Database,

    #[error("unique constraint violation")]
    Duplicate,

    #[error("repository unavailable")]
    Unavailable,

    #[error("repository operation failed")]
    OperationFailed,

    #[error("stored data is invalid: {0}")]
    InvalidData(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_errors_describe_themselves() {
        assert_eq!(RepositoryError::Database.to_string(), "database error");
        assert_eq!(
            RepositoryError::Duplicate.to_string(),
            "unique constraint violation"
        );
        assert_eq!(
            RepositoryError::Unavailable.to_string(),
            "repository unavailable"
        );
        assert_eq!(
            RepositoryError::OperationFailed.to_string(),
            "repository operation failed"
        );
        assert_eq!(
            RepositoryError::InvalidData("bad".into()).to_string(),
            "stored data is invalid: bad"
        );
        assert!(format!("{:?}", RepositoryError::Database).contains("Database"));
    }
}
