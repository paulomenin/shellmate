//! `init <shell>`: print the glue script.

use super::finish_text;
use crate::init;
use shellmate_api::Error;
use shellmate_core::Config;

pub(crate) fn run(config: Option<&std::path::Path>, shell: &str) -> i32 {
    finish_text(Config::load(config).and_then(|cfg| {
        let bin =
            std::env::current_exe().map_err(|e| Error::new("init.bad_path", e.to_string()))?;
        init::generate(shell, &cfg, &bin.to_string_lossy())
    }))
}
