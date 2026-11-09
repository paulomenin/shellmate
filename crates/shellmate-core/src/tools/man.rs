//! The `man` tool.

use super::call::ToolCall;
use super::text::{clean_man, truncate};
use std::time::Duration;

/// Run an allowed `man` call and return text for the model. Failures are results too (the model can
/// carry on without the page), never errors.
pub async fn run_man(call: &ToolCall, max_bytes: usize) -> String {
    run_man_with("man", call, max_bytes).await
}

/// [`run_man`] with the program named, so tests can stand in for `man`.
pub(super) async fn run_man_with(program: &str, call: &ToolCall, max_bytes: usize) -> String {
    use tokio::process::Command;
    let mut cmd = Command::new(program);
    cmd.args(&call.args)
        .env("MANPAGER", "cat")
        .env("PAGER", "cat")
        .env("MANWIDTH", "100")
        .env("GROFF_NO_SGR", "1")
        .env_remove("MANOPT")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let out =
        match tokio::time::timeout(crate::limits::scaled(Duration::from_secs(5)), cmd.output())
            .await
        {
            Err(_) => return format!("`{}` timed out", call.display()),
            Ok(Err(e)) => return format!("could not run man: {e}"),
            Ok(Ok(o)) => o,
        };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.lines().next().unwrap_or("").trim();
        return if err.is_empty() {
            format!("`{}` failed", call.display())
        } else {
            format!("`{}` failed: {err}", call.display())
        };
    }
    let text = clean_man(&String::from_utf8_lossy(&out.stdout));
    truncate(&text, max_bytes)
}
