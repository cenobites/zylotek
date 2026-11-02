use async_trait::async_trait;

pub mod tokio;

#[async_trait]
pub trait EventPublisher<E>: Send + Sync {
    async fn publish(&self, event: E) -> Result<(), EventPublishError>;
}

#[async_trait]
pub trait EventHandler<E>: Send + Sync {
    async fn handle(&self, event: E) -> Result<(), EventHandlerError>;
}

#[derive(Debug, thiserror::Error)]
pub enum EventHandlerError {
    #[error("failed to handle the event")]
    Failed,
}

#[derive(Debug, thiserror::Error)]
pub enum EventPublishError {
    #[error("no subscribers")]
    NoSubscribers,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_errors_describe_themselves() {
        assert_eq!(
            EventPublishError::NoSubscribers.to_string(),
            "no subscribers"
        );
        assert_eq!(
            EventHandlerError::Failed.to_string(),
            "failed to handle the event"
        );
        assert!(format!("{:?}", EventHandlerError::Failed).contains("Failed"));
    }
}
