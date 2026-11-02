use thiserror::Error;

use crate::errors::RepositoryError;

#[derive(Debug, Error)]
pub enum WoodError {
    #[error("wood scientific name '{scientific_name}' already exists")]
    ScientificNameAlreadyExists { scientific_name: String },

    #[error("{field} must not be empty")]
    EmptyText { field: &'static str },

    #[error("scientific name '{0}' must have a genus and a specific epithet")]
    InvalidScientificName(String),

    #[error("invalid measurement: {value} {unit}")]
    InvalidMeasurement { unit: &'static str, value: f64 },

    #[error("invalid url '{0}'")]
    InvalidUrl(String),

    #[error("invalid wood id '{0}'")]
    InvalidId(String),

    #[error("repository error")]
    Repository(#[from] RepositoryError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wood_errors_describe_themselves() {
        let duplicate = WoodError::ScientificNameAlreadyExists {
            scientific_name: "Astronium lecointei".into(),
        };
        assert_eq!(
            duplicate.to_string(),
            "wood scientific name 'Astronium lecointei' already exists"
        );

        let repository = WoodError::from(RepositoryError::Database);
        assert_eq!(repository.to_string(), "repository error");
        assert!(format!("{repository:?}").contains("Database"));
    }
}
