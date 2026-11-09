//! Tests that need a program from the machine (git, a shell) ask for it here.
//!
//! A missing program skips the test with a message, unless `REQUIRE_TOOLS` is set (CI sets it):
//! then it fails, so a runner without git cannot report green while testing nothing.

/// True when `program` is on `PATH`. Panics instead of returning false when `REQUIRE_TOOLS` is set.
pub(crate) fn have(program: &str) -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    if crate::command::resolve_program(program, &path).is_some() {
        return true;
    }
    assert!(
        std::env::var_os("REQUIRE_TOOLS").is_none(),
        "REQUIRE_TOOLS is set but `{program}` is not installed"
    );
    eprintln!("skipping: `{program}` is not installed");
    false
}
