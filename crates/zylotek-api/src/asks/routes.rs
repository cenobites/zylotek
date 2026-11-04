use axum::{
    Router,
    routing::{get, post},
};

use crate::{
    asks::handlers::{create_ask_handler, get_ask_handler, record_progress_handler},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_ask_handler))
        .route("/{id}", get(get_ask_handler))
        .route("/{id}/progress", post(record_progress_handler))
}
