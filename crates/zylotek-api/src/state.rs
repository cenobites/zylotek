use std::sync::Arc;

use zylotek_application::{asks::facade::AskFacade, woods::facade::WoodFacade};

use crate::auth::Auth;

#[derive(Clone)]
pub struct AppState {
    pub woods: Arc<WoodFacade>,
    pub asks: Arc<AskFacade>,
    pub auth: Auth,
}
