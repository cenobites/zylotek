//! Use cases for asks: open one, read one, and record each step of answering it. The
//! answering itself happens elsewhere (the web app drives the assistant); these only keep
//! the record, refuse steps the state machine does not allow, and publish an `AskEvent` per
//! change.

pub mod clock;
pub mod create_ask;
pub mod errors;
pub mod facade;
pub mod get_ask;
pub mod handlers;
pub mod record_ask_progress;

#[cfg(test)]
pub(crate) mod support;
