//! The generated glue must at least parse in the shell it is meant for.

mod common;

use common::*;
use std::io::Write;
use std::process::{Command, Stdio};

fn syntax_ok(shell: &str, args: &[&str], script: &str) -> Result<(), String> {
    let mut child = Command::new(shell)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[test]
fn generated_glue_parses_in_each_shell() {
    let cfg = "version = 1\n[input.shortcuts]\n\"alt+m\" = \"ask\"\n\"alt+i\" = \"generate\"\n\"alt+j\" = \"explain\"\n\"ctrl+alt+x\" = \"ask\"\n";
    let mut checked = 0;
    for (shell, args) in [
        ("fish", vec!["--no-execute"]),
        ("zsh", vec!["-n"]),
        ("bash", vec!["-n"]),
    ] {
        if !have_tool(shell) {
            continue;
        }
        let r = run(&["init", shell], cfg, b"");
        assert_eq!(r.code, 0, "{shell}: {}", r.stderr);
        syntax_ok(shell, &args, &r.stdout).unwrap_or_else(|e| {
            panic!("{shell} cannot parse the generated glue: {e}\n{}", r.stdout)
        });
        checked += 1;
    }
    assert!(checked > 0, "no shell available to check");
}
