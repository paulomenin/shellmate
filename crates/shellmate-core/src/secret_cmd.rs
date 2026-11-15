//! `cmd:` secret references: ask a program for the secret (`pass show x`, `op read ...`, `secret-tool ...`).
//!
//! The line after `cmd:` is split into arguments by [`split`], which accepts no shell syntax at all, and the
//! program is started directly (never through a shell). The program must be an absolute path or a bare
//! name found in `PATH`; a relative path is refused, so a directory cannot supply it. It runs with no
//! stdin, a small environment (see [`BASE_ENV`]), a timeout and capped output, and what it printed on
//! stdout is the secret.
//!
//! Messages name the program and the config key, never the whole line (an argument may hold a token) and
//! never the output.
//!
//! Unlike `exec`, the child stays in shellmate's process group: a keyring or GnuPG pinentry that asks for a
//! passphrase on the terminal needs to be in the foreground group to read it. On a timeout the child itself
//! is killed.

use crate::ports::{Env, Host};
use crate::sanitize;
use crate::secrets::expand_home;
use shellmate_api::{Error, Result};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The most a secret program may print. Over this is an error, not a truncated key.
pub const MAX_OUTPUT: usize = 4096;

/// Variables the program always gets (when set): where it lives, who runs it, and where a keyring or
/// GnuPG agent can be reached. None of them holds a secret. Everything else comes from `[secrets] pass_env`.
pub const BASE_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TMPDIR",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "DBUS_SESSION_BUS_ADDRESS",
    "GNUPGHOME",
    "GPG_TTY",
];

/// `[secrets]` as the runner needs it.
#[derive(Debug, Clone)]
pub struct CmdOptions {
    pub timeout: Duration,
    /// Extra variables to pass on (`PASSWORD_STORE_DIR`, `OP_SERVICE_ACCOUNT_TOKEN`, ...).
    pub pass_env: Vec<String>,
}

impl Default for CmdOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            pass_env: Vec::new(),
        }
    }
}

fn invalid(key: &str, why: impl std::fmt::Display) -> Error {
    Error::new("config.invalid_cmd", format!("`{key}`: cmd: line {why}")).with_hint(
        "write the program and its arguments only; quote with '...' or \"...\"; no pipes, \
         redirects, variables or globs (put those in a script and call the script)",
    )
}

/// Characters that mean something to a shell. Outside quotes (and, for `$` and the backtick, inside double
/// quotes) they are an error, so a line never behaves differently from what it looks like.
const SHELL_SYNTAX: &str = "|&;<>()$`*?[]{}!#\\";

/// Split a `cmd:` line into arguments. A leading `~/` of an unquoted word becomes the home directory.
/// The reason is returned without echoing the line.
pub fn split(key: &str, line: &str, env: &dyn Env) -> Result<Vec<String>> {
    let mut args: Vec<String> = Vec::new();
    let mut cur = String::new();
    let (mut in_word, mut tilde) = (false, false);
    let mut chars = line.chars().peekable();
    let finish =
        |args: &mut Vec<String>, cur: &mut String, in_word: &mut bool, tilde: &mut bool| {
            if *in_word {
                let word = std::mem::take(cur);
                args.push(if *tilde {
                    expand_home(&word, env).to_string_lossy().into_owned()
                } else {
                    word
                });
            }
            *in_word = false;
            *tilde = false;
        };
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => finish(&mut args, &mut cur, &mut in_word, &mut tilde),
            c if c.is_control() => {
                return Err(invalid(key, "has a control character or a line break"))
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) if c.is_control() => {
                            return Err(invalid(key, "has a control character or a line break"))
                        }
                        Some(c) => cur.push(c),
                        None => return Err(invalid(key, "has an unclosed single quote")),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\')) => cur.push(e),
                            _ => return Err(invalid(key, "has a backslash that is not `\\\"` or `\\\\`")),
                        },
                        Some(c @ ('$' | '`')) => {
                            return Err(invalid(key, format!("has `{c}` in double quotes (nothing is expanded; use single quotes)")))
                        }
                        Some(c) if c.is_control() => {
                            return Err(invalid(key, "has a control character or a line break"))
                        }
                        Some(c) => cur.push(c),
                        None => return Err(invalid(key, "has an unclosed double quote")),
                    }
                }
            }
            c if SHELL_SYNTAX.contains(c) => {
                return Err(invalid(key, format!("has shell syntax (`{c}`)")))
            }
            '~' if !in_word && chars.peek() == Some(&'/') => {
                in_word = true;
                tilde = true;
                cur.push('~');
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    finish(&mut args, &mut cur, &mut in_word, &mut tilde);
    if args.is_empty() || args[0].is_empty() {
        return Err(invalid(key, "names no program"));
    }
    Ok(args)
}

fn is_executable(host: &Host, path: &std::path::Path) -> bool {
    host.fs
        .metadata(path)
        .map(|m| m.is_file && m.mode & 0o111 != 0)
        .unwrap_or(false)
}

/// The program to start: an absolute path as written, or a bare name found in `PATH`. A path with a
/// directory part that is not absolute is refused, and so are relative and empty `PATH` entries (they mean
/// "here").
pub fn find_program(key: &str, argv0: &str, host: &Host) -> Result<PathBuf> {
    let missing = |why: &str| {
        Error::new(
            "config.secret_cmd_missing",
            format!("`{key}`: program `{argv0}` {why}"),
        )
        .with_hint("install it, or write its absolute path")
    };
    if argv0.contains('/') {
        let path = PathBuf::from(argv0);
        if !path.is_absolute() {
            return Err(Error::new(
                "config.invalid_cmd",
                format!("`{key}`: program `{argv0}` is a relative path"),
            )
            .with_hint("use an absolute path (or `~/...`), or a bare name found in PATH"));
        }
        return if is_executable(host, &path) {
            Ok(path)
        } else {
            Err(missing("is not an executable file"))
        };
    }
    let path_var = host.env.var("PATH").unwrap_or_default();
    path_var
        .split(':')
        .filter(|d| d.starts_with('/'))
        .map(|d| std::path::Path::new(d).join(argv0))
        .find(|p| is_executable(host, p))
        .ok_or_else(|| missing("was not found in PATH"))
}

/// Everything `config check` can know without running the program: the line splits and the program exists.
pub fn check(key: &str, line: &str, host: &Host) -> Result<()> {
    let args = split(key, line, host.env.as_ref())?;
    find_program(key, &args[0], host).map(|_| ())
}

/// An argument that looks like a key somebody pasted into the line: a well-known prefix, or a long run of
/// letters and digits. A `cmd:` line is meant to name where a secret lives, not to hold it.
fn looks_like_a_secret(arg: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "sk-",
        "ghp_",
        "gho_",
        "github_pat_",
        "xox",
        "AKIA",
        "eyJ",
        "glpat-",
        "AIza",
    ];
    PREFIXES
        .iter()
        .any(|p| arg.starts_with(p) && arg.len() >= p.len() + 8)
        || (arg.len() >= 32
            && arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && arg.chars().any(|c| c.is_ascii_digit())
            && arg.chars().any(|c| c.is_ascii_alphabetic()))
}

/// Whether an argument of the line looks like a secret written in place. A line that does not split is
/// not judged here (it fails elsewhere).
pub fn has_embedded_secret(line: &str, env: &dyn Env) -> bool {
    split("", line, env)
        .map(|args| args.iter().skip(1).any(|a| looks_like_a_secret(a)))
        .unwrap_or(false)
}

/// The line as `config show` prints it: as written, except that a line with a secret in it shows only the
/// program, so showing the config never shows the key.
pub fn display(line: &str, env: &dyn Env) -> String {
    if has_embedded_secret(line, env) {
        let program = line.split_whitespace().next().unwrap_or("");
        format!("{program} (arguments hidden: one looks like a secret)")
    } else {
        line.to_string()
    }
}

enum Piece {
    Out(Vec<u8>),
    Err(Vec<u8>),
    Done,
}

fn pump(mut r: impl Read + Send + 'static, tx: mpsc::Sender<Piece>, out: bool) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 {
                break;
            }
            let piece = buf[..n].to_vec();
            if tx
                .send(if out {
                    Piece::Out(piece)
                } else {
                    Piece::Err(piece)
                })
                .is_err()
            {
                return;
            }
        }
        let _ = tx.send(Piece::Done);
    });
}

/// How long to keep reading after the program has exited, for a helper that still holds the pipes. Like the
/// other short internal limits it follows `SHELLMATE_TIME_SCALE`, so a slow or instrumented run does not cut
/// off output that is only late.
fn grace_period() -> Duration {
    crate::limits::scaled(Duration::from_millis(300))
}

/// Run the `cmd:` line and return its single line of output, without the line break.
pub fn run(key: &str, line: &str, host: &Host, opts: &CmdOptions) -> Result<String> {
    let args = split(key, line, host.env.as_ref())?;
    let program = find_program(key, &args[0], host)?;
    let name = args[0].rsplit('/').next().unwrap_or(&args[0]).to_string();
    let failed = |why: String| {
        Error::new(
            "config.secret_cmd_failed",
            format!("`{key}`: `{name}` {why}"),
        )
    };
    let mut cmd = Command::new(&program);
    cmd.arg0(&args[0]).args(&args[1..]).env_clear();
    for var in BASE_ENV
        .iter()
        .copied()
        .chain(opts.pass_env.iter().map(String::as_str))
    {
        if let Some(v) = host.env.var(var) {
            cmd.env(var, v);
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| failed(format!("could not start: {e}")))?;
    let (tx, rx) = mpsc::channel();
    pump(child.stdout.take().expect("piped"), tx.clone(), true);
    pump(child.stderr.take().expect("piped"), tx, false);

    let deadline = Instant::now() + opts.timeout;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let (mut done, mut status, mut grace) = (0, None, None::<Instant>);
    let kill = |child: &mut std::process::Child| {
        let _ = child.kill();
        let _ = child.wait();
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Piece::Out(b)) => out.extend(b),
            Ok(Piece::Err(b)) => {
                if err.len() < MAX_OUTPUT {
                    err.extend(b);
                }
            }
            Ok(Piece::Done) => done += 1,
            Err(_) => {}
        }
        if out.len() > MAX_OUTPUT {
            kill(&mut child);
            return Err(failed(format!("printed more than {MAX_OUTPUT} bytes")));
        }
        if status.is_none() {
            if let Ok(Some(st)) = child.try_wait() {
                status = Some(st);
                // a helper the program left behind may keep the pipes open: wait a moment, not forever, and
                // never past the timeout: the program is done, so what is left is only stragglers
                grace = Some((Instant::now() + grace_period()).min(deadline));
            }
        }
        if status.is_some() && (done == 2 || grace.is_some_and(|g| Instant::now() > g)) {
            break;
        }
        if Instant::now() > deadline {
            kill(&mut child);
            return Err(Error::new(
                "config.secret_cmd_failed",
                format!(
                    "`{key}`: `{name}` did not finish in {}s",
                    opts.timeout.as_secs_f32()
                ),
            )
            .with_hint(
                "raise `[secrets] cmd_timeout`, or check that the program is not waiting for input",
            ));
        }
    }
    let status = status.expect("loop ends with a status");
    if !status.success() {
        let code = status
            .code()
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| "killed by a signal".into());
        let detail: String = String::from_utf8_lossy(&err)
            .trim()
            .chars()
            .take(200)
            .collect();
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(": {}", sanitize::escape(&detail))
        };
        return Err(failed(format!("failed ({code}){detail}")));
    }
    let text =
        String::from_utf8(out).map_err(|_| failed("printed text that is not UTF-8".into()))?;
    let value = text.trim_end_matches(['\n', '\r']);
    if value.is_empty() {
        return Err(Error::new(
            "config.secret_unset",
            format!("`{key}`: `{name}` printed nothing"),
        ));
    }
    if value.contains(|c: char| c.is_control()) {
        return Err(failed("printed more than one line".into()).with_hint(
            "the command must print only the key; for `pass`, keep the key alone in that entry, or \
             call a small script that prints the first line",
        ));
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests;
