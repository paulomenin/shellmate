//! Shared helpers for the binary tests.
#![allow(dead_code)]

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn config(url: &str) -> String {
    format!(
        "version = 1\n[providers.mock]\ntrust = \"local\"\nendpoint = \"{url}\"\n[models.mock]\nprovider = \"mock\"\nplugin = \"builtin:openai-compatible\"\nmodel = \"test-model\"\n[llm]\nchain = [\"mock\"]\n"
    )
}

pub fn envelope(action: &str, buffer: &str) -> Vec<u8> {
    format!("v=1\0action={action}\0shell=fish\0cwd=/tmp\0cursor=0\0buffer={buffer}\0").into_bytes()
}

pub fn run(args: &[&str], config_text: &str, stdin: &[u8]) -> Run {
    run_with(args, config_text, stdin, &[])
}

/// Like [`run`] with extra environment variables for the child.
pub fn run_with(args: &[&str], config_text: &str, stdin: &[u8], envs: &[(&str, &str)]) -> Run {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sm-e2e-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.toml");
    std::fs::write(&cfg, config_text).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_shellmate"));
    cmd.arg("--config")
        .arg(&cfg)
        .args(args)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .envs(envs.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // detach from the controlling terminal so the client falls back to stderr instead of writing to the developer's tty
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    std::fs::remove_dir_all(dir).ok();
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into(),
        stderr: String::from_utf8_lossy(&out.stderr).into(),
    }
}

pub fn completion(content: &str) -> String {
    serde_json::json!({"choices":[{"message":{"role":"assistant","content":content},"finish_reason":"stop"}]}).to_string()
}

/// A config with one *remote* provider at the mock URL, allowed context level 2.
pub fn remote_config(url: &str, extra: &str) -> String {
    format!(
        "version = 1\n[context]\ndefault_level = 2\n[providers.cloud]\ntrust = \"remote\"\nmax_context_level = 2\nendpoint = \"{url}\"\n[models.cloud]\nprovider = \"cloud\"\nplugin = \"builtin:openai-compatible\"\nmodel = \"m\"\n[llm]\nchain = [\"cloud\"]\n{extra}"
    )
}

/// Envelope with history entries (`<status>\t<command>`).
pub fn envelope_with_history(action: &str, buffer: &str, hist: &[(&str, &str)]) -> Vec<u8> {
    let mut e =
        format!("v=1\0action={action}\0shell=fish\0cwd=/home/someone/projects/x\0status=1\0");
    for (st, cmd) in hist {
        e.push_str(&format!("hist={st}\t{cmd}\0"));
    }
    e.push_str(&format!("buffer={buffer}\0"));
    e.into_bytes()
}

/// A private scratch directory that is removed on drop.
pub struct Scratch(pub std::path::PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sm-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    pub fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
    pub fn str(&self, name: &str) -> String {
        self.path(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// `config(url)` plus a store directory and any extra TOML.
pub fn conv_config(url: &str, store_dir: &str, extra: &str) -> String {
    format!(
        "{}[plugins.\"builtin:store-tmpfs\"]\ndir = \"{store_dir}\"\n{extra}\n",
        config(url)
    )
}

/// An envelope that belongs to a shell session.
pub fn envelope_s(action: &str, buffer: &str, session: &str) -> Vec<u8> {
    format!(
        "v=1\0action={action}\0shell=fish\0cwd=/tmp\0cursor=0\0session={session}\0buffer={buffer}\0"
    )
    .into_bytes()
}

/// Envelope with a session and history entries.
pub fn envelope_session_history(
    action: &str,
    buffer: &str,
    session: &str,
    hist: &[(&str, &str)],
) -> Vec<u8> {
    let mut e = format!(
        "v=1\0action={action}\0shell=fish\0cwd=/home/someone/projects/x\0status=1\0session={session}\0"
    );
    for (st, cmd) in hist {
        e.push_str(&format!("hist={st}\t{cmd}\0"));
    }
    e.push_str(&format!("buffer={buffer}\0"));
    e.into_bytes()
}

/// True when `program` is on `PATH`. A missing one skips the test with a message, or fails it when
/// `REQUIRE_TOOLS` is set (CI), so a runner without it cannot report green while testing nothing.
#[allow(dead_code)]
pub fn have_tool(program: &str) -> bool {
    let found = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
        .unwrap_or(false);
    if !found {
        assert!(
            std::env::var_os("REQUIRE_TOOLS").is_none(),
            "REQUIRE_TOOLS is set but `{program}` is not installed"
        );
        eprintln!("skipping: `{program}` is not installed");
    }
    found
}
