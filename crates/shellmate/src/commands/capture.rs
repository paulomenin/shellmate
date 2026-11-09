//! The `capture` action: wrap the input line in `shellmate run --`, or unwrap it.

use crate::commands::EXIT_UNCHANGED;
use crate::tty;
use shellmate_api::Error;
use shellmate_core::Envelope;
use std::io::Write;

/// The `capture` action: wrap the input line in `shellmate run --`, or unwrap it. Local text only: no
/// provider, no network, and nothing is run.
pub(crate) fn run(tty: &tty::Tty, env: &Envelope) -> Result<i32, Error> {
    use shellmate_core::capture::{quote_for, wrap_line, Wrapped};
    let exe = std::env::current_exe().ok();
    let path = std::env::var("PATH").unwrap_or_default();
    let on_path = shellmate_core::command::resolve_program("shellmate", &path)
        .zip(exe.as_ref())
        .is_some_and(|(a, b)| a.canonicalize().ok() == b.canonicalize().ok());
    let bin = match (&exe, on_path) {
        (_, true) => "shellmate".to_string(),
        (Some(e), false) => {
            let e = e.to_string_lossy();
            if e.chars()
                .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
            {
                e.into_owned()
            } else {
                quote_for(&env.shell, &e)
            }
        }
        (None, false) => "shellmate".to_string(),
    };
    match wrap_line(&env.buffer, &env.shell, &bin) {
        Wrapped::Line(line) => {
            let mut out = std::io::stdout().lock();
            out.write_all(line.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| Error::new("glue.protocol", format!("writing stdout: {e}")))?;
            Ok(0)
        }
        Wrapped::Refused(why) => {
            tty.write(&format!("{why}\n"));
            Ok(EXIT_UNCHANGED)
        }
    }
}
