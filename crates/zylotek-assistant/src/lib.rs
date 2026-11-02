//! The wood assistant: answers questions such as "which wood for a table with Japanese
//! joinery for 12 seats?" from the catalogue of `zylotek-api`, using an open model served by
//! llama.cpp (or any OpenAI-compatible server) only to read the question and to explain.

pub mod assistant;
pub mod catalogue;
pub mod error;
pub mod http;
pub mod llm;
pub mod ranking;
pub mod requirements;
pub mod server;

pub use assistant::{Assistant, DEFAULT_TOP};
