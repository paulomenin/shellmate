//! shellmate core: configuration, glue envelope, prompt/response handling and the pipeline.

pub mod action;
pub mod audit;
pub mod capture;
pub mod collect;
pub mod command;
pub mod config;
pub mod conversation;
pub mod engine;
pub mod envelope;
pub mod limits;
#[cfg(test)]
mod needs;
pub mod ports;
pub mod present;
pub mod prompt;
pub mod render;
pub mod response;
pub mod sanitize;
pub mod secret_cmd;
pub mod secrets;
pub mod tools;
pub mod validate;

pub use config::Config;
pub use engine::{Action, Engine, Outcome};
pub use envelope::Envelope;
