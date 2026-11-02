//! The Zylotek website: server-rendered pages to ask which wood suits a project, browse the
//! catalogue and compare woods side by side. It talks to `zylotek-api` and
//! `zylotek-assistant` over HTTP using the shared contract types, and never touches the
//! database itself.

pub mod clients;
pub mod error;
pub mod pages;
pub mod server;
pub mod view;
