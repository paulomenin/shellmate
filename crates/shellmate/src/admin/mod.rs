//! Everything around the request path: `config show/init`, `install-shell`, `setup`, `doctor`, `audit`.
//!
//! One module per command; this file holds what they share.

mod audit_cmd;
mod config_cmd;
mod doctor;
mod install_shell;
mod setup;
mod templates;

pub use audit_cmd::audit_cmd;
pub use config_cmd::{config_init, config_show};
pub use doctor::doctor;
pub use install_shell::install_shell;
pub use setup::{setup, SetupArgs};

use shellmate_core::ports::Env;
use shellmate_core::Config;
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn config_path(explicit: Option<&Path>, env: &dyn Env) -> PathBuf {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| Config::default_path_in(env))
        .unwrap_or_else(|| home().join(".config/shellmate/config.toml"))
}
