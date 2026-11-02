use zylotek_domain::woods::events::WoodEvent;
use zylotek_infrastructure::event_bus::{EventHandler, EventHandlerError};

pub struct WoodEventHandler;

impl WoodEventHandler {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for WoodEventHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl EventHandler<WoodEvent> for WoodEventHandler {
    async fn handle(&self, event: WoodEvent) -> Result<(), EventHandlerError> {
        match event {
            WoodEvent::WoodCreated { wood_id } => log::info!("wood {wood_id} was created"),
            WoodEvent::WoodUpdated { wood_id } => log::info!("wood {wood_id} was updated"),
            WoodEvent::WoodDeleted { wood_id } => log::info!("wood {wood_id} was deleted"),
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use zylotek_domain::woods::entities::WoodId;

    use super::*;

    #[tokio::test]
    async fn handles_every_event_kind() {
        let handler = <WoodEventHandler as Default>::default();
        let wood_id = WoodId::new();

        for event in [
            WoodEvent::WoodCreated { wood_id },
            WoodEvent::WoodUpdated { wood_id },
            WoodEvent::WoodDeleted { wood_id },
        ] {
            handler.handle(event).await.unwrap();
        }
    }

    #[test]
    fn new_and_default_agree() {
        let _ = WoodEventHandler::new();
        let _ = <WoodEventHandler as Default>::default();
    }
}
