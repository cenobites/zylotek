//! The API's wire format, shared by the server and its clients so they cannot drift apart.

pub mod errors;
pub mod oauth;
pub mod woods;

pub use errors::ErrorResponse;
