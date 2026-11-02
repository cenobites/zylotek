use std::sync::Arc;

use zylotek_application::woods::facade::WoodFacade;

use crate::auth::Auth;

#[derive(Clone)]
pub struct AppState {
    pub woods: Arc<WoodFacade>,
    pub auth: Auth,
}
