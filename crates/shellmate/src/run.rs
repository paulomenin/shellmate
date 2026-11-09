//! `shellmate run -- <command>`: run a command exactly as usual and keep a bounded copy of its output for the
//! next request (see `shellmate_core::capture`). It never contacts a provider and never changes how the
//! command behaves, except that its output goes through pipes (programs that check for a terminal may drop
//! colors or interactive features, so wrap non-interactive commands).

use shellmate_core::capture::{self, Capture};
use shellmate_core::ports::{Clock, SystemClock, SystemEnv};
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const SHELLS: &[&str] = &["fish", "zsh", "bash", "sh", "dash"];

/// Keeps the start and the end of what went by, and counts the middle that was dropped.
struct Tee {
    head: Vec<u8>,
    tail: Vec<u8>,
    omitted: usize,
}

impl Tee {
    const HEAD: usize = capture::STORE_MAX / 3;
    const TAIL: usize = capture::STORE_MAX - Self::HEAD;

    fn new() -> Self {
        Self {
            head: Vec::new(),
            tail: Vec::new(),
            omitted: 0,
        }
    }

    fn push(&mut self, mut chunk: &[u8]) {
        if self.head.len() < Self::HEAD {
            let take = (Self::HEAD - self.head.len()).min(chunk.len());
            self.head.extend_from_slice(&chunk[..take]);
            chunk = &chunk[take..];
        }
        self.tail.extend_from_slice(chunk);
        if self.tail.len() > Self::TAIL * 2 {
            let drop = self.tail.len() - Self::TAIL;
            self.tail.drain(..drop);
            self.omitted += drop;
        }
    }

    /// `(text, cut)`
    fn finish(mut self) -> (String, bool) {
        if self.tail.len() > Self::TAIL {
            let drop = self.tail.len() - Self::TAIL;
            self.tail.drain(..drop);
            self.omitted += drop;
        }
        let head = capture::clean(&self.head);
        let tail = capture::clean(&self.tail);
        if self.omitted == 0 {
            // one stream cleaned twice would split a line at the seam: join the raw bytes instead
            let mut all = self.head;
            all.extend_from_slice(&self.tail);
            (capture::clean(&all), false)
        } else {
            (
                format!("{head}\n[... {} bytes omitted ...]\n{tail}", self.omitted),
                true,
            )
        }
    }
}

fn pump(mut from: impl Read, mut to: impl Write, tee: Arc<Mutex<Tee>>) {
    let mut buf = [0u8; 8192];
    // a closed output (`| head`) must not stop the capture or leave the child blocked on a full pipe
    let mut out_ok = true;
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        if out_ok && to.write_all(&buf[..n]).and_then(|_| to.flush()).is_err() {
            out_ok = false;
        }
        tee.lock().expect("tee lock").push(&buf[..n]);
    }
}

pub fn run(shell: Option<&str>, args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("shellmate run: no command given (usage: shellmate run -- <command> [args...])");
        return 2;
    }
    let (mut cmd, shown) = match shell {
        Some(sh) => {
            if !SHELLS.contains(&sh) {
                eprintln!(
                    "shellmate run: unknown shell `{sh}` (expected one of: {})",
                    SHELLS.join(", ")
                );
                return 2;
            }
            let line = args.join(" ");
            let mut c = Command::new(sh);
            c.arg("-c").arg(&line);
            (c, line)
        }
        None => {
            let mut c = Command::new(&args[0]);
            c.args(&args[1..]);
            (c, args.join(" "))
        }
    };
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let name = shell.unwrap_or(&args[0]);
            eprintln!("shellmate run: {name}: {e}");
            return if e.kind() == std::io::ErrorKind::NotFound {
                127
            } else {
                126
            };
        }
    };
    // Ctrl-C goes to the whole foreground group: the command stops, and we stay to save what it printed.
    // SAFETY: setting a signal disposition has no memory-safety preconditions.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
    let tee = Arc::new(Mutex::new(Tee::new()));
    let out = child.stdout.take().expect("piped stdout");
    let err = child.stderr.take().expect("piped stderr");
    let t1 = {
        let tee = tee.clone();
        std::thread::spawn(move || pump(out, std::io::stdout(), tee))
    };
    let t2 = {
        let tee = tee.clone();
        std::thread::spawn(move || pump(err, std::io::stderr(), tee))
    };
    let status = child.wait();
    let _ = t1.join();
    let _ = t2.join();
    let code = match &status {
        Ok(s) => s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0)),
        Err(e) => {
            eprintln!("shellmate run: waiting for the command: {e}");
            1
        }
    };

    match std::env::var("SHELLMATE_SESSION") {
        Ok(session) if !session.is_empty() => {
            let tee = Arc::try_unwrap(tee)
                .map(|m| m.into_inner().expect("tee lock"))
                .unwrap_or_else(|_| Tee::new());
            let (output, truncated) = tee.finish();
            let cap = Capture {
                command: shown,
                status: Some(code),
                at: SystemClock.unix_secs(),
                truncated,
                output,
            };
            if let Err(e) = capture::save(&SystemEnv, &session, &cap) {
                eprintln!("shellmate run: the output was not saved: {}", e.message);
            }
        }
        _ => eprintln!(
            "shellmate run: the output was not saved (no shell session; load the shell integration, `shellmate init`)"
        ),
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tee_keeps_head_and_tail_of_big_output() {
        let mut t = Tee::new();
        t.push(b"START\n");
        for _ in 0..400 {
            t.push(&[b'x'; 1024]);
            t.push(b"\n");
        }
        t.push(b"END\n");
        let (text, cut) = t.finish();
        assert!(cut);
        assert!(text.starts_with("START") && text.trim_end().ends_with("END"));
        assert!(text.contains("bytes omitted"));
        assert!(text.len() < capture::STORE_MAX + 100);
    }

    #[test]
    fn small_output_is_whole() {
        let mut t = Tee::new();
        t.push(b"a\n");
        t.push(b"b\n");
        assert_eq!(t.finish(), ("a\nb".to_string(), false));
    }
}
