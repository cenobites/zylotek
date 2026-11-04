//! `/api/v1/asks`: questions kept as records while the web app answers them in the background.
//! Reading one is public (its id is the key to it); opening one and recording its steps needs a
//! token with the `asks:write` scope.

pub mod handlers;
pub mod routes;
