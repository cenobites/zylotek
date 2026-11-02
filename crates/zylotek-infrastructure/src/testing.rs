//! Test support: event publishers and an in-memory database.
//!
//! Compiled for this crate's tests and, behind the `testing` feature, for other crates'
//! tests and benches. Never enable the feature in a production dependency.

use std::sync::Mutex;

use crate::{
    errors::InfrastructureError,
    event_bus::{EventPublishError, EventPublisher},
    persistence::build_db,
};

/// An [`EventPublisher`] that records every event it is given.
pub struct RecordingEventPublisher<E> {
    events: Mutex<Vec<E>>,
}

impl<E> Default for RecordingEventPublisher<E> {
    fn default() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl<E: Clone> RecordingEventPublisher<E> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<E> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl<E: Send + Sync> EventPublisher<E> for RecordingEventPublisher<E> {
    async fn publish(&self, event: E) -> Result<(), EventPublishError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

/// An [`EventPublisher`] that always fails, like a bus nobody listens to.
pub struct FailingEventPublisher;

#[async_trait::async_trait]
impl<E: Send + Sync + 'static> EventPublisher<E> for FailingEventPublisher {
    async fn publish(&self, _event: E) -> Result<(), EventPublishError> {
        Err(EventPublishError::NoSubscribers)
    }
}

/// A fresh, migrated, in-memory database.
pub async fn memory_db() -> Result<toasty::Db, InfrastructureError> {
    build_db("sqlite::memory:").await
}
