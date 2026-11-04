//! A question asked of the assistant, kept as a record that moves through a fixed set of
//! states while it is answered in the background, so it can be looked up again by its id.

pub mod entities;
pub mod errors;
pub mod events;
pub mod repositories;
pub mod services;
pub mod value_objects;
