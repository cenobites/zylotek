use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    UnprocessableEntity(String),

    #[error("{0}")]
    BadRequest(String),

    #[error("internal server error")]
    Internal(#[source] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),

            ApiError::UnprocessableEntity(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY")
            }

            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),

            ApiError::Internal(error) => {
                tracing::error!(error = ?error, "internal server error");
                (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR")
            }
        };

        let body = Json(ErrorResponse {
            code,
            message: self.to_string(),
        });

        (status, body).into_response()
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: &'static str,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use serde_json::Value;

    use super::*;

    async fn render(error: ApiError) -> (StatusCode, Value) {
        let response = error.into_response();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();

        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn each_error_maps_to_its_status_and_code() {
        let cases = [
            (
                ApiError::NotFound("missing".into()),
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                "missing",
            ),
            (
                ApiError::UnprocessableEntity("invalid".into()),
                StatusCode::UNPROCESSABLE_ENTITY,
                "UNPROCESSABLE_ENTITY",
                "invalid",
            ),
            (
                ApiError::BadRequest("bad".into()),
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "bad",
            ),
        ];

        for (error, status, code, message) in cases {
            let (actual_status, body) = render(error).await;

            assert_eq!(actual_status, status);
            assert_eq!(body["code"], code);
            assert_eq!(body["message"], message);
        }
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_their_cause() {
        let (status, body) =
            render(ApiError::Internal(anyhow::anyhow!("secret db password"))).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], "INTERNAL_ERROR");
        assert_eq!(body["message"], "internal server error");
        assert!(!body.to_string().contains("secret"));
    }

    #[test]
    fn errors_expose_their_source_and_debug_form() {
        let error = ApiError::Internal(anyhow::anyhow!("cause"));

        assert!(std::error::Error::source(&error).is_some());
        assert!(format!("{error:?}").contains("Internal"));
        assert!(
            format!(
                "{:?}",
                ErrorResponse {
                    code: "X",
                    message: "m".into()
                }
            )
            .contains("code")
        );
    }
}
