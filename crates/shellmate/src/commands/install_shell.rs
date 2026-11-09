//! `install-shell`: the shell defaults to the login shell.

use super::finish_text;
use crate::admin;
use shellmate_api::Error;

pub(crate) fn run(shell: Option<String>, yes: bool, remove: bool, dry_run: bool) -> i32 {
    finish_text(
        shell
            .or_else(|| {
                std::env::var("SHELL")
                    .ok()
                    .and_then(|s| s.rsplit('/').next().map(String::from))
            })
            .ok_or_else(|| {
                Error::new("setup.no_shell", "cannot tell which shell")
                    .with_hint("pass fish, zsh or bash")
            })
            .and_then(|sh| admin::install_shell(&sh, yes, remove, dry_run)),
    )
}
