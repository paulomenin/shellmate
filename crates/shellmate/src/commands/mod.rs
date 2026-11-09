//! One module per command. `main` parses the command line and calls one of these; each returns the
//! process exit code.

pub(crate) mod audit;
pub(crate) mod capture;
pub(crate) mod check;
pub(crate) mod config_check;
pub(crate) mod context;
pub(crate) mod conversation;
pub(crate) mod install_shell;
pub(crate) mod invoke;
pub(crate) mod render;
pub(crate) mod shell_init;
pub(crate) mod ui;

use crate::tty;
use shellmate_api::Error;
use shellmate_core::present;

/// `invoke` told the glue to leave the buffer as it is.
pub(crate) const EXIT_UNCHANGED: i32 = 10;
pub(crate) const EXIT_ERROR: i32 = 1;

/// Print a command's text result, or its error, and return the exit code.
pub(crate) fn finish_text(result: Result<String, Error>) -> i32 {
    match result {
        Ok(out) => {
            print!("{out}");
            0
        }
        Err(e) => {
            eprint!("{}", ui::stderr().cli_error(&e));
            EXIT_ERROR
        }
    }
}

pub(crate) fn report(tty: &tty::Tty, e: &Error) {
    tty.write(&present::presenter(ui::tty_caps(tty)).error(e));
}
