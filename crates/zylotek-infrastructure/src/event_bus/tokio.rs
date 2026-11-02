use tokio::sync::broadcast;

use crate::event_bus::{EventHandler, EventPublishError, EventPublisher};

pub struct TokioEventPublisher<E> {
    sender: broadcast::Sender<E>,
}

impl<E> TokioEventPublisher<E>
where
    E: Clone + Send + Sync + 'static,
{
    pub fn new(buffer: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer);

        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<E> {
        self.sender.subscribe()
    }
}

#[async_trait::async_trait]
impl<E> EventPublisher<E> for TokioEventPublisher<E>
where
    E: Clone + Send + Sync + 'static,
{
    async fn publish(&self, event: E) -> Result<(), EventPublishError> {
        self.sender
            .send(event)
            .map(|_| ())
            .map_err(|_| EventPublishError::NoSubscribers)
    }
}

pub async fn run_event_handler<E, H>(mut receiver: broadcast::Receiver<E>, handler: H)
where
    E: Clone + Send + Sync + 'static,
    H: EventHandler<E>,
{
    loop {
        match receiver.recv().await {
            Ok(event) => {
                if let Err(error) = handler.handle(event).await {
                    tracing::error!(?error, "event handler failed");
                }
            }

            Err(broadcast::error::RecvError::Lagged(count)) => {
                tracing::error!(count, "event handler lagged");
            }

            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!("event channel closed");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::event_bus::EventHandlerError;

    #[derive(Clone)]
    struct Recorder {
        seen: Arc<Mutex<Vec<u32>>>,
        fail_on: Option<u32>,
    }

    #[async_trait::async_trait]
    impl EventHandler<u32> for Recorder {
        async fn handle(&self, event: u32) -> Result<(), EventHandlerError> {
            self.seen.lock().unwrap().push(event);
            if self.fail_on == Some(event) {
                return Err(EventHandlerError::Failed);
            }
            Ok(())
        }
    }

    fn recorder(fail_on: Option<u32>) -> Recorder {
        Recorder {
            seen: Arc::default(),
            fail_on,
        }
    }

    #[tokio::test]
    async fn publish_without_subscribers_fails() {
        let publisher = TokioEventPublisher::<u32>::new(4);

        let error = publisher.publish(1).await.unwrap_err();

        assert!(matches!(error, EventPublishError::NoSubscribers));
    }

    #[tokio::test]
    async fn every_subscriber_receives_each_event() {
        let publisher = TokioEventPublisher::<u32>::new(4);
        let mut first = publisher.subscribe();
        let mut second = publisher.subscribe();

        publisher.publish(7).await.unwrap();

        assert_eq!(first.recv().await.unwrap(), 7);
        assert_eq!(second.recv().await.unwrap(), 7);
    }

    #[tokio::test]
    async fn the_handler_processes_events_until_the_channel_closes() {
        let publisher = TokioEventPublisher::<u32>::new(8);
        let handler = recorder(None);
        let seen = handler.seen.clone();
        let task = tokio::spawn(run_event_handler(publisher.subscribe(), handler));

        for event in 1..=3 {
            publisher.publish(event).await.unwrap();
        }
        drop(publisher);
        task.await.unwrap();

        assert_eq!(*seen.lock().unwrap(), [1, 2, 3]);
    }

    #[tokio::test]
    async fn a_failing_handler_does_not_stop_the_loop() {
        let publisher = TokioEventPublisher::<u32>::new(8);
        let handler = recorder(Some(2));
        let seen = handler.seen.clone();
        let task = tokio::spawn(run_event_handler(publisher.subscribe(), handler));

        for event in 1..=3 {
            publisher.publish(event).await.unwrap();
        }
        drop(publisher);
        task.await.unwrap();

        assert_eq!(*seen.lock().unwrap(), [1, 2, 3]);
    }

    #[tokio::test]
    async fn a_lagging_handler_skips_ahead_and_keeps_running() {
        let publisher = TokioEventPublisher::<u32>::new(2);
        let receiver = publisher.subscribe();
        // Overflow the buffer before the handler starts so its first recv reports lag.
        for event in 1..=5 {
            publisher.publish(event).await.unwrap();
        }
        let handler = recorder(None);
        let seen = handler.seen.clone();
        let task = tokio::spawn(run_event_handler(receiver, handler));
        drop(publisher);
        task.await.unwrap();

        assert_eq!(*seen.lock().unwrap(), [4, 5]);
    }
}
