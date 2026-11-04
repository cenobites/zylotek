use zylotek_domain::asks::events::AskEvent;
use zylotek_infrastructure::event_bus::{EventHandler, EventHandlerError};

pub struct AskEventHandler;

impl AskEventHandler {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for AskEventHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl EventHandler<AskEvent> for AskEventHandler {
    async fn handle(&self, event: AskEvent) -> Result<(), EventHandlerError> {
        match event {
            AskEvent::AskOpened { ask_id } => log::info!("ask {ask_id} was opened"),
            AskEvent::AskStarted { ask_id } => log::info!("ask {ask_id} was started"),
            AskEvent::AskRanked { ask_id } => log::info!("ask {ask_id} was ranked"),
            AskEvent::AskExplaining { ask_id } => log::info!("ask {ask_id} is being explained"),
            // One per saved batch of the explanation: too chatty for info.
            AskEvent::AskExplained { ask_id } => log::debug!("ask {ask_id} was explained further"),
            AskEvent::AskAnswered { ask_id, complete } => {
                log::info!("ask {ask_id} was answered (complete: {complete})")
            }
            AskEvent::AskFailed { ask_id } => log::warn!("ask {ask_id} failed"),
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use zylotek_domain::asks::entities::AskId;

    use super::*;

    #[tokio::test]
    async fn handles_every_event_kind() {
        let handler = <AskEventHandler as Default>::default();
        let ask_id = AskId::new();

        for event in [
            AskEvent::AskOpened { ask_id },
            AskEvent::AskStarted { ask_id },
            AskEvent::AskRanked { ask_id },
            AskEvent::AskExplaining { ask_id },
            AskEvent::AskExplained { ask_id },
            AskEvent::AskAnswered {
                ask_id,
                complete: false,
            },
            AskEvent::AskFailed { ask_id },
        ] {
            handler.handle(event).await.unwrap();
        }
    }

    #[test]
    fn new_and_default_agree() {
        let _ = AskEventHandler::new();
        let _ = <AskEventHandler as Default>::default();
    }
}
