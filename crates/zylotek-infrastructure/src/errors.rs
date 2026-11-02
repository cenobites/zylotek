#[derive(Debug, thiserror::Error)]
pub enum InfrastructureError {
    #[error("database error: {0}")]
    Database(#[from] anyhow::Error),

    #[error("fatal error: {0}")]
    Fatal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infrastructure_errors_describe_themselves() {
        let database = InfrastructureError::from(anyhow::anyhow!("boom"));
        assert_eq!(database.to_string(), "database error: boom");

        let fatal = InfrastructureError::Fatal("no runtime".into());
        assert_eq!(fatal.to_string(), "fatal error: no runtime");
        assert!(format!("{fatal:?}").contains("Fatal"));
    }
}
