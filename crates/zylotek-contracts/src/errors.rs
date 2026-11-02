use serde::{Deserialize, Serialize};

/// The body of every error response from the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// A stable machine-readable code such as `NOT_FOUND`.
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_body_is_a_code_and_a_message() {
        let body = ErrorResponse {
            code: "NOT_FOUND".into(),
            message: "wood 1 not found".into(),
        };

        let json = serde_json::to_value(&body).unwrap();

        assert_eq!(
            json,
            serde_json::json!({"code": "NOT_FOUND", "message": "wood 1 not found"})
        );
        assert_eq!(serde_json::from_value::<ErrorResponse>(json).unwrap(), body);
        assert!(format!("{:?}", body.clone()).contains("NOT_FOUND"));
    }
}
