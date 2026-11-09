//! The approval prompt and the terminal handling, driven through a real pseudo-terminal.
//!
//! The binary is started as the session leader of a fresh pty, so `/dev/tty` is the pty. The test plays the
//! user: it waits for the prompt text, presses keys, and then checks, from what the mock model was told,
//! whether the command ran. Nothing here is mocked on the terminal side.

mod common;

use common::*;
use shellmate_testing::MockHttpServer;
use std::io::Write;
use std::os::fd::RawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Pty {
    master: RawFd,
    slave: RawFd,
    seen: String,
}

impl Pty {
    fn open() -> Self {
        unsafe {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC);
            assert!(master >= 0, "posix_openpt");
            assert_eq!(libc::grantpt(master), 0);
            assert_eq!(libc::unlockpt(master), 0);
            let mut name = [0 as libc::c_char; 128];
            assert_eq!(libc::ptsname_r(master, name.as_mut_ptr(), name.len()), 0);
            let slave = libc::open(
                name.as_ptr(),
                libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
            );
            assert!(slave >= 0, "open slave");
            Self {
                master,
                slave,
                seen: String::new(),
            }
        }
    }

    fn set_width(&self, cols: u16) {
        let ws = libc::winsize {
            ws_row: 40,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe { libc::ioctl(self.master, libc::TIOCSWINSZ, &ws) };
    }

    fn send(&self, bytes: &[u8]) {
        let n = unsafe { libc::write(self.master, bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(n, bytes.len() as isize, "write to the pty");
    }

    /// Read whatever the program has written so far, waiting up to `ms`.
    fn pump(&mut self, ms: i32) -> bool {
        let mut p = libc::pollfd {
            fd: self.master,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut p, 1, ms) } <= 0 {
            return false;
        }
        let mut buf = [0u8; 4096];
        let n = unsafe { libc::read(self.master, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            return false; // hang-up: the program has gone
        }
        self.seen
            .push_str(&String::from_utf8_lossy(&buf[..n as usize]));
        true
    }

    /// Wait until the transcript contains `needle`; panics with the transcript when it never does.
    fn wait_for(&mut self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.seen.contains(needle) {
            assert!(
                Instant::now() < deadline,
                "never saw {needle:?}; the terminal showed {:?}",
                self.seen
            );
            self.pump(100);
        }
    }
}

impl Pty {
    /// Type like a person: one key per read, so a run of bytes is not mistaken for a paste.
    fn type_keys(&self, keys: &[u8]) {
        for k in keys {
            self.send(&[*k]);
            std::thread::sleep(Duration::from_millis(60));
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.master);
            libc::close(self.slave);
        }
    }
}

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
    screen: String,
}

/// Runs `shellmate invoke` with the pty as its controlling terminal, on a terminal that cannot show colour
/// (`TERM=dumb`), so what is on the screen is the text and nothing else. `typeahead` is typed before the
/// program starts; `play` then acts as the user.
fn in_pty(
    cfg: &str,
    envelope: &[u8],
    typeahead: &[u8],
    width: u16,
    play: impl FnOnce(&mut Pty),
) -> Outcome {
    in_pty_with(cfg, envelope, typeahead, width, &[("TERM", "dumb")], play)
}

/// Like [`in_pty`], with these environment variables for the program. Variables that decide colour are
/// cleared first, so a developer's own settings cannot change a test.
fn in_pty_with(
    cfg: &str,
    envelope: &[u8],
    typeahead: &[u8],
    width: u16,
    envs: &[(&str, &str)],
    play: impl FnOnce(&mut Pty),
) -> Outcome {
    let dir = std::env::temp_dir().join(format!(
        "sm-pty-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let cfgfile = dir.join("config.toml");
    std::fs::write(&cfgfile, cfg).unwrap();
    let mut pty = Pty::open();
    pty.set_width(width);
    if !typeahead.is_empty() {
        pty.send(typeahead);
    }
    let slave = pty.slave;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_shellmate"));
    cmd.arg("--config")
        .arg(&cfgfile)
        .arg("invoke")
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .envs(envs.iter().copied())
        .env("LANG", "en_US.UTF-8")
        .env("HOME", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            if libc::ioctl(slave, libc::TIOCSCTTY, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(envelope).unwrap();
    play(&mut pty);
    // let it finish, collecting what it prints; a program that hangs fails the test
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        pty.pump(50);
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(
            Instant::now() < deadline,
            "shellmate did not finish; screen: {:?}",
            pty.seen
        );
    };
    while pty.pump(20) {}
    let (mut o, mut e) = (String::new(), String::new());
    std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut o).ok();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut e).ok();
    std::fs::remove_dir_all(&dir).ok();
    Outcome {
        code: status.code().unwrap_or(-1),
        stdout: o,
        stderr: e,
        screen: std::mem::take(&mut pty.seen),
    }
}

/// A model that asks to run `command`, then answers once it has either the output or a refusal.
async fn model_running(command: &'static str) -> MockHttpServer {
    MockHttpServer::start_dynamic(200, move |req| {
        if req.contains("chose not to let you") {
            completion("Run it yourself.")
        } else if req.contains("exit status") || req.contains("was denied") {
            completion("Understood.\n```shellmate-command\nls\n```")
        } else {
            completion(&format!("```shellmate-tool\nexec {command}\n```"))
        }
    })
    .await
}

fn envelope_in(cwd: &str, buffer: &str) -> Vec<u8> {
    format!("v=1\0action=ask\0shell=bash\0cwd={cwd}\0cursor=0\0buffer={buffer}\0").into_bytes()
}

/// Did the model get the command's output (it ran) rather than a refusal (it did not)?
async fn ran(llm: &MockHttpServer) -> bool {
    let last = llm.last_request().await;
    assert!(
        last.contains("exit status")
            || last.contains("was denied")
            || last.contains("chose not to let you"),
        "the model was never told the outcome: {last}"
    );
    last.contains("exit status")
}

macro_rules! ask {
    ($command:expr, $typeahead:expr, $width:expr, |$p:ident| $play:block) => {{
        let llm = model_running($command).await;
        let cfg = config(&llm.url());
        let e = envelope_in("/tmp", "check something");
        let out =
            tokio::task::spawn_blocking(move || in_pty(&cfg, &e, $typeahead, $width, |$p| $play))
                .await
                .unwrap();
        (llm, out)
    }};
}

const LOW: &str = "choice (n): ";
const HIGH: &str = "answer: ";

#[tokio::test(flavor = "multi_thread")]
async fn y_runs_the_command_and_the_screen_says_so() {
    let (llm, out) = ask!("echo approve-me", b"", 100, |p| {
        p.wait_for(LOW);
        p.send(b"y");
    });
    assert!(ran(&llm).await, "{}", out.screen);
    assert!(out.screen.contains("echo approve-me"), "{}", out.screen);
    assert!(out.screen.contains("choice (n): run"), "{}", out.screen);
    assert_eq!(out.code, 0, "{}", out.stderr);
}

#[tokio::test(flavor = "multi_thread")]
async fn anything_but_y_or_b_denies() {
    // Enter, a lone Esc, Ctrl-C, Ctrl-D, n, and an unrelated letter
    for key in [&b"\r"[..], b"\x1b", b"\x03", b"\x04", b"n", b"q", b" "] {
        let (llm, out) = ask!("echo approve-me", b"", 100, |p| {
            p.wait_for(LOW);
            p.send(key);
        });
        assert!(!ran(&llm).await, "key {key:?} approved: {}", out.screen);
        assert!(out.screen.contains("denied"), "key {key:?}: {}", out.screen);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn b_puts_the_command_in_the_prompt_without_running_it() {
    let (llm, out) = ask!("echo approve-me", b"", 100, |p| {
        p.wait_for(LOW);
        p.send(b"b");
    });
    assert!(!ran(&llm).await);
    assert_eq!(out.stdout, "echo approve-me");
    assert!(out.screen.contains("to your prompt"), "{}", out.screen);
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_typed_before_the_prompt_cannot_answer_it() {
    // a "y" and an Enter typed while the model was still thinking
    let (llm, out) = ask!("echo approve-me", b"y\ry\r", 100, |p| {
        p.wait_for(LOW);
        p.send(b"n");
    });
    assert!(
        !ran(&llm).await,
        "typeahead approved the command: {}",
        out.screen
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pasted_text_and_escape_sequences_are_not_answers() {
    // an arrow key, then a paste of "yy" (two bytes in one read), then a real n
    let (llm, out) = ask!("echo approve-me", b"", 100, |p| {
        p.wait_for(LOW);
        p.send(b"\x1b[A");
        std::thread::sleep(Duration::from_millis(250));
        p.send(b"yy");
        std::thread::sleep(Duration::from_millis(250));
        p.send(b"n");
    });
    assert!(!ran(&llm).await, "{}", out.screen);
    // and an arrow key does not stop a later single y from counting
    let (llm, out) = ask!("echo approve-me", b"", 100, |p| {
        p.wait_for(LOW);
        p.send(b"\x1b[A");
        std::thread::sleep(Duration::from_millis(250));
        p.send(b"y");
    });
    assert!(ran(&llm).await, "{}", out.screen);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_high_risk_command_needs_the_whole_word() {
    let cmd = "rm /nonexistent/shellmate-approval-test";
    // the same keys that approve a low-risk command do not approve this one
    for keys in [
        &b"y"[..],
        b"y\r",
        b"ye\r",
        b"yess\r",
        b"YES\r",
        b"\r",
        b"\x03",
        b"n\r",
    ] {
        let (llm, out) = ask!(cmd, b"", 100, |p| {
            p.wait_for(HIGH);
            p.type_keys(keys);
            // a bare "y" is not an answer, so the prompt keeps waiting: give up like a user would
            std::thread::sleep(Duration::from_millis(300));
            p.send(b"\x03");
        });
        assert!(!ran(&llm).await, "keys {keys:?} approved: {}", out.screen);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_high_risk_command_runs_when_yes_is_typed_and_backspace_edits() {
    let cmd = "rm /nonexistent/shellmate-approval-test";
    for keys in [&b"yes\r"[..], b"yex\x7fs\r", b"yesx\x7f\r"] {
        let (llm, out) = ask!(cmd, b"", 100, |p| {
            p.wait_for(HIGH);
            p.type_keys(keys);
        });
        assert!(ran(&llm).await, "keys {keys:?}: {}", out.screen);
    }
    // "b" + Enter hands the command to the prompt instead
    let (llm, out) = ask!(cmd, b"", 100, |p| {
        p.wait_for(HIGH);
        p.type_keys(b"b\r");
    });
    assert!(!ran(&llm).await);
    assert_eq!(out.stdout, cmd);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hung_up_terminal_denies_and_does_not_spin() {
    let started = Instant::now();
    let (llm, _out) = ask!("echo approve-me", b"", 100, |p| {
        p.wait_for(LOW);
        // the terminal goes away (an ssh session drops): the master side closes
        unsafe { libc::close(std::mem::replace(&mut p.master, -1)) };
    });
    assert!(!ran(&llm).await);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_command_is_wrapped_to_the_terminal_width_and_shown_whole() {
    let long = "echo aaaaaaaaaabbbbbbbbbbccccccccccddddddddddeeeeeeeeeeffffffffffgggggggggg";
    let (_llm, out) = ask!(long, b"", 40, |p| {
        p.wait_for(LOW);
        p.send(b"n");
    });
    let words = out.screen.replace(['\r', '\n', ' ', '\u{2502}', '|'], "");
    assert!(
        words.contains(&long.replace(' ', "")),
        "the command was cut: {}",
        out.screen
    );
    assert!(
        out.screen.lines().all(|l| l.chars().count() <= 90),
        "an unwrapped line: {}",
        out.screen
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_enter_pressed_while_waiting_is_discarded_and_reported() {
    // no tools: a plain answer, with a line typed before it arrived
    let llm = MockHttpServer::start(200, &completion("Fine.\n```shellmate-command\nls\n```")).await;
    let cfg = config(&llm.url());
    let e = envelope_in("/tmp", "list files");
    let out =
        tokio::task::spawn_blocking(move || in_pty(&cfg, &e, b"rm -rf important\r", 100, |_| {}))
            .await
            .unwrap();
    assert!(
        out.screen
            .contains("Enter pressed while waiting was ignored"),
        "{}",
        out.screen
    );
    assert_eq!(out.stdout, "ls");
}

/// A model that asks to read `hello.txt` in a scratch directory, then answers once it has the text or a refusal.
async fn read_scenario(keys: &'static [u8]) -> (bool, Outcome) {
    let d = std::env::temp_dir().join(format!("sm-pty-read-{}-{}", std::process::id(), keys.len()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("hello.txt"), "file-body-marker\n").unwrap();
    let llm = MockHttpServer::start_dynamic(200, |req| {
        if req.contains("file-body-marker") || req.contains("was denied") {
            completion("Done.")
        } else {
            completion("```shellmate-tool\nread hello.txt\n```")
        }
    })
    .await;
    let cfg = config(&llm.url());
    let e = envelope_in(&d.display().to_string(), "what is in it");
    let out = tokio::task::spawn_blocking(move || {
        in_pty(&cfg, &e, b"", 100, |p| {
            p.wait_for(LOW);
            p.send(keys);
        })
    })
    .await
    .unwrap();
    let body_reached_model = llm.last_request().await.contains("file-body-marker");
    std::fs::remove_dir_all(&d).ok();
    (body_reached_model, out)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_read_prompt_reads_on_y_and_never_offers_the_prompt_option() {
    let (read, out) = read_scenario(b"y").await;
    assert!(read, "{}", out.screen);
    assert!(!out.screen.contains("[b]"), "{}", out.screen);
    // `b` is not an answer for a read: it denies
    let (read, out) = read_scenario(b"b").await;
    assert!(!read, "{}", out.screen);
    assert!(out.screen.contains("denied"), "{}", out.screen);
}

/// `strip` without its styles (`ESC [ digits and ; m`).
fn strip_styles(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\u{1b}' && b.get(i + 1) == Some(&'[') {
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == ';') {
                j += 1;
            }
            if b.get(j) == Some(&'m') {
                i = j + 1;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn on_a_colour_terminal_the_prompt_is_coloured_and_answers_the_same() {
    let llm = model_running("echo approve-me").await;
    let cfg = config(&llm.url());
    let e = envelope_in("/tmp", "check something");
    let out = tokio::task::spawn_blocking(move || {
        in_pty_with(&cfg, &e, b"", 100, &[("TERM", "xterm-256color")], |p| {
            p.wait_for(LOW);
            p.send(b"y");
        })
    })
    .await
    .unwrap();
    assert!(ran(&llm).await, "{}", out.screen);
    // the risk is green, the keys are coloured, and the text under the colours is the usual prompt
    assert!(
        out.screen.contains("\u{1b}[1m\u{1b}[32mlow"),
        "{:?}",
        out.screen
    );
    assert!(out.screen.contains("\u{1b}[32m[y]"), "{:?}", out.screen);
    let text = strip_styles(&out.screen);
    assert!(
        text.contains("risk: low") && text.contains("[y] run") && text.contains("echo approve-me"),
        "{text}"
    );
    assert!(text.contains("choice (n): run"), "{text}");
    assert_eq!(out.code, 0, "{}", out.stderr);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_colour_is_shown_on_a_dumb_terminal_or_with_no_color_set() {
    for envs in [
        vec![("TERM", "dumb")],
        vec![("TERM", "xterm"), ("NO_COLOR", "1")],
    ] {
        let llm = model_running("echo approve-me").await;
        let cfg = config(&llm.url());
        let e = envelope_in("/tmp", "check something");
        let out = tokio::task::spawn_blocking(move || {
            in_pty_with(&cfg, &e, b"", 100, &envs, |p| {
                p.wait_for(LOW);
                p.send(b"n");
            })
        })
        .await
        .unwrap();
        let colour = out.screen.split("\u{1b}[").skip(1).any(|s| {
            let params: String = s
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == ';')
                .collect();
            s[params.len()..].starts_with('m')
                && params.split(';').any(|n| {
                    n.parse::<u32>()
                        .is_ok_and(|n| (30..=37).contains(&n) || (90..=97).contains(&n))
                })
        });
        assert!(!colour, "no colour was asked for: {:?}", out.screen);
    }
}
