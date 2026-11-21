//! The approval prompt (`approval-ux.md` §4.1): a model-driven command runs only after a human says so, on
//! the terminal, with one key. This is the only implementation of `Approver`; no plugin can answer it.
//!
//! Rules, all deliberate:
//! * Only a single-byte read of `y` approves. Longer reads (arrow keys, escape sequences, pasted text) and
//!   every other key mean no. Enter, Esc, Ctrl-C, Ctrl-D, a timeout, a signal and a closed terminal deny.
//! * Keys typed before the prompt appeared are thrown away, so they cannot answer it.
//! * The command is shown whole and escaped: control characters and bidi tricks are made visible.
//! * High risk needs the word `yes` and Enter.
//! * No terminal: deny.

use shellmate_core::command::Risk;
use shellmate_core::present::{self, Caps, Presenter, Tone};
use shellmate_core::sanitize;
use shellmate_core::tools::{ApprovalKind, ApprovalRequest, Approver, Choice};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static SIGNALED: AtomicBool = AtomicBool::new(false);
extern "C" fn on_signal(_: libc::c_int) {
    SIGNALED.store(true, Ordering::SeqCst);
}

pub struct TtyApprover {
    caps: Caps,
    timeout: Duration,
}

impl TtyApprover {
    pub fn new(caps: Caps) -> Self {
        Self {
            caps,
            timeout: Duration::from_secs(120),
        }
    }
}

struct Raw {
    fd: i32,
    saved: libc::termios,
    old_actions: Vec<(libc::c_int, libc::sigaction)>,
}

impl Raw {
    fn open() -> Option<Self> {
        let fd = unsafe {
            libc::open(
                c"/dev/tty".as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOCTTY,
            )
        };
        if fd < 0 {
            return None;
        }
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut t) } != 0 {
            unsafe { libc::close(fd) };
            return None;
        }
        let saved = t;
        t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        t.c_iflag &= !(libc::IXON | libc::ICRNL | libc::INLCR);
        t.c_cc[libc::VMIN] = 1;
        t.c_cc[libc::VTIME] = 0;
        // TCSAFLUSH also discards pending input.
        unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &t) };
        SIGNALED.store(false, Ordering::SeqCst);
        let mut old_actions = Vec::new();
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            let mut new: libc::sigaction = unsafe { std::mem::zeroed() };
            new.sa_sigaction = on_signal as *const () as usize;
            let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
            unsafe { libc::sigaction(sig, &new, &mut old) };
            old_actions.push((sig, old));
        }
        Some(Self {
            fd,
            saved,
            old_actions,
        })
    }

    fn write(&self, s: &str) {
        let b = s.as_bytes();
        let mut off = 0;
        while off < b.len() {
            let n = unsafe { libc::write(self.fd, b[off..].as_ptr().cast(), b.len() - off) };
            if n <= 0 {
                break;
            }
            off += n as usize;
        }
    }

    fn width(&self) -> usize {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(self.fd, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
            usize::from(ws.ws_col)
        } else {
            80
        }
    }

    fn flush_input(&self) {
        unsafe { libc::tcflush(self.fd, libc::TCIFLUSH) };
    }

    fn wait(&self, ms: i32) -> bool {
        let mut p = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe {
            libc::poll(&mut p, 1, ms) > 0 && (p.revents & (libc::POLLIN | libc::POLLHUP)) != 0
        }
    }

    fn read(&self, buf: &mut [u8]) -> isize {
        unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) }
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        unsafe {
            for (sig, old) in &self.old_actions {
                libc::sigaction(*sig, old, std::ptr::null_mut());
            }
            libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.saved);
            libc::close(self.fd);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Key {
    Char(u8),
    Enter,
    Backspace,
    /// Anything that must never count as an answer: Esc, Ctrl-C, Ctrl-D, end of input, a signal, a timeout.
    Abort,
    /// Escape sequences and pasted text: ignored, not an answer.
    Ignored,
}

/// What one `read(2)` returned, as a key. Only a read of exactly one byte can be a character.
fn key_from_bytes(b: &[u8], lone_esc: bool) -> Key {
    match b {
        [] => Key::Abort,
        [b'\r'] | [b'\n'] => Key::Enter,
        [0x7f] | [0x08] => Key::Backspace,
        [0x03] | [0x04] => Key::Abort,
        [0x1b] if lone_esc => Key::Abort,
        [0x1b] => Key::Ignored,
        [c] => Key::Char(*c),
        _ => Key::Ignored,
    }
}

fn read_key(t: &Raw, deadline: Instant) -> Key {
    loop {
        if SIGNALED.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return Key::Abort;
        }
        if !t.wait(100) {
            continue;
        }
        let mut b = [0u8; 64];
        let n = t.read(&mut b);
        if n == 0 {
            return Key::Abort;
        }
        if n < 0 {
            continue;
        }
        let b = &b[..n as usize];
        let lone_esc = b == [0x1b] && !t.wait(40);
        if b == [0x1b] && !lone_esc {
            // the start of a sequence: swallow the rest
            let mut rest = [0u8; 64];
            t.read(&mut rest);
        }
        return key_from_bytes(b, lone_esc);
    }
}

/// `text` cut into lines that start with `first` and then `next`. `indent` is how many columns `first`
/// takes on screen, which may be fewer than its characters when it carries styling.
fn wrap(text: &str, width: usize, indent: usize, first: &str, next: &str) -> String {
    let w = width.saturating_sub(indent + 1).max(20);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return first.to_string();
    }
    chars
        .chunks(w)
        .enumerate()
        .map(|(i, c)| {
            format!(
                "{}{}",
                if i == 0 { first } else { next },
                c.iter().collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// The prompt text. Everything from the model or the command is escaped. Pure, so it is tested. The command
/// itself is never styled, so it is read exactly as it will run.
fn render(req: &ApprovalRequest, width: usize, utf8: bool, p: &dyn Presenter) -> String {
    let (tl, v, bl, h) = if utf8 {
        ("╭─", "│", "╰─", "─")
    } else {
        ("+-", "|", "+-", "-")
    };
    let dim = |s: &str| p.label(Tone::Muted, s);
    let (tl_s, v_s, bl_s) = (dim(tl), dim(v), dim(bl));
    let read = req.kind == ApprovalKind::Read;
    let title = if read {
        " shellmate wants to read a file "
    } else {
        " shellmate wants to run a command "
    };
    let rule = h.repeat(width.saturating_sub(title.chars().count() + 4).clamp(3, 60));
    let mut o = format!("{tl_s}{}{}\r\n", p.label(Tone::Info, title), dim(&rule));
    let cmd = sanitize::escape(&req.command);
    let lead = if read { "read: " } else { "$ " };
    o.push_str(&wrap(
        &cmd,
        width,
        v.chars().count() + 2 + lead.chars().count(),
        &format!("{v_s}  {lead}"),
        &format!("{v_s}    "),
    ));
    o.push_str("\r\n");
    if let Some(cwd) = &req.cwd {
        o.push_str(&format!("{v_s}  in: {}\r\n", sanitize::escape(cwd)));
    }
    if !read {
        let tone = match req.risk {
            Risk::Low => Tone::Success,
            Risk::Medium => Tone::Warning,
            Risk::High => Tone::Error,
        };
        o.push_str(&format!(
            "{v_s}  risk: {}\r\n",
            p.label(tone, req.risk.label())
        ));
    }
    for r in &req.reasons {
        o.push_str(&wrap(
            &sanitize::escape(r),
            width,
            v.chars().count() + 2 + 2,
            &format!("{v_s}  {} ", p.label(Tone::Warning, "!")),
            &format!("{v_s}    "),
        ));
        o.push_str("\r\n");
    }
    o.push_str(&format!("{v_s}\r\n"));
    let key = |tone, k: &str| p.label(tone, k);
    if read {
        o.push_str(&format!(
            "{v_s}  {} read it   {} deny (Enter, Esc and Ctrl-C deny)\r\n",
            key(Tone::Success, "[y]"),
            key(Tone::Error, "[n]")
        ));
        o.push_str(&format!("{bl_s} choice (n): "));
    } else if req.risk == Risk::High {
        let buffer = if req.can_buffer {
            ", b + Enter to put it in your prompt"
        } else {
            ""
        };
        o.push_str(&format!(
            "{v_s}  type {} + Enter to run it{buffer}; anything else denies\r\n",
            p.label(Tone::Error, "yes")
        ));
        o.push_str(&format!("{bl_s} answer: "));
    } else {
        let buffer = if req.can_buffer {
            format!("   {} put it in my prompt", key(Tone::Info, "[b]"))
        } else {
            String::new()
        };
        o.push_str(&format!(
            "{v_s}  {} run   {} deny{buffer} (Enter, Esc and Ctrl-C deny)\r\n",
            key(Tone::Success, "[y]"),
            key(Tone::Error, "[n]")
        ));
        o.push_str(&format!("{bl_s} choice (n): "));
    }
    o
}

impl Approver for TtyApprover {
    fn approve(&self, req: &ApprovalRequest) -> Choice {
        let Some(t) = Raw::open() else {
            return Choice::Deny; // no terminal: deny
        };
        let deadline = Instant::now() + self.timeout;
        // clear a "thinking…" line, then make sure nothing typed earlier can answer
        t.write("\r\x1b[K\r\n");
        t.flush_input();
        let p = present::presenter(self.caps);
        t.write(&render(req, t.width(), self.caps.unicode, &*p));
        let choice = if req.risk == Risk::High {
            let mut typed = String::new();
            loop {
                match read_key(&t, deadline) {
                    Key::Char(c) if c.is_ascii_graphic() && typed.len() < 8 => {
                        typed.push(c as char);
                        t.write(&(c as char).to_string());
                    }
                    Key::Backspace => {
                        if typed.pop().is_some() {
                            t.write("\x08 \x08");
                        }
                    }
                    Key::Enter => {
                        break match typed.as_str() {
                            "yes" => Choice::Run,
                            "b" if req.can_buffer => Choice::ToBuffer,
                            _ => Choice::Deny,
                        }
                    }
                    Key::Ignored | Key::Char(_) => {}
                    Key::Abort => break Choice::Deny,
                }
            }
        } else {
            loop {
                match read_key(&t, deadline) {
                    Key::Char(b'y') => break Choice::Run,
                    Key::Char(b'b') if req.kind == ApprovalKind::Run && req.can_buffer => {
                        break Choice::ToBuffer
                    }
                    Key::Ignored => continue,
                    _ => break Choice::Deny,
                }
            }
        };
        let (tone, said) = match choice {
            Choice::Run if req.kind == ApprovalKind::Read => (Tone::Success, "read"),
            Choice::Run => (Tone::Success, "run"),
            Choice::Deny => (Tone::Error, "denied"),
            Choice::ToBuffer => (Tone::Info, "to your prompt"),
        };
        t.write(&format!("{}\r\n", p.label(tone, said)));
        choice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> shellmate_core::present::PlainPresenter {
        shellmate_core::present::PlainPresenter::new(Caps::plain(80, true))
    }

    fn colour() -> Box<dyn Presenter> {
        present::presenter(Caps {
            width: 80,
            unicode: true,
            level: shellmate_core::present::Level::Ansi16,
            markdown: true,
        })
    }

    fn req(cmd: &str, risk: Risk) -> ApprovalRequest {
        ApprovalRequest {
            kind: ApprovalKind::Run,
            command: cmd.into(),
            cwd: Some("/home/u/x".into()),
            risk,
            reasons: vec!["`rm` modifies or destroys files".into()],
            can_buffer: true,
        }
    }

    #[test]
    fn a_file_read_prompt_has_no_run_options_and_no_risk() {
        let r = ApprovalRequest {
            kind: ApprovalKind::Read,
            command: "src/main.rs".into(),
            cwd: Some("/home/u/x".into()),
            risk: Risk::Low,
            reasons: vec![
                "120 bytes; the text is sent to `cloud` (remote, after redaction)".into(),
            ],
            can_buffer: false,
        };
        let out = render(&r, 80, true, &plain());
        assert!(
            out.contains("wants to read a file") && out.contains("read: src/main.rs"),
            "{out}"
        );
        assert!(
            out.contains("sent to `cloud`") && out.contains("[y] read it"),
            "{out}"
        );
        assert!(
            !out.contains("risk:") && !out.contains("[b]") && !out.contains("run"),
            "{out}"
        );
    }

    #[test]
    fn only_a_single_byte_is_a_character() {
        assert_eq!(key_from_bytes(b"y", false), Key::Char(b'y'));
        assert_eq!(key_from_bytes(b"yy", false), Key::Ignored);
        assert_eq!(
            key_from_bytes(b"y\n", false),
            Key::Ignored,
            "pasted text is never an answer"
        );
        assert_eq!(key_from_bytes(b"\x1b[A", false), Key::Ignored);
        assert_eq!(key_from_bytes(b"\x1b", true), Key::Abort);
        assert_eq!(key_from_bytes(b"\x03", false), Key::Abort);
        assert_eq!(key_from_bytes(b"\x04", false), Key::Abort);
        assert_eq!(key_from_bytes(b"", false), Key::Abort);
        assert_eq!(key_from_bytes(b"\r", false), Key::Enter);
    }

    #[test]
    fn the_command_is_shown_whole_and_escaped() {
        let r = req("echo \u{1b}[2J\u{202e}evil", Risk::Low);
        let s = render(&r, 80, true, &plain());
        assert!(!s.contains('\u{1b}') && !s.contains('\u{202e}'), "{s:?}");
        assert!(s.contains("\\u{1b}[2J") && s.contains("\\u{202e}"));
        let long = "x".repeat(300);
        let s = render(&req(&long, Risk::Low), 60, true, &plain());
        let in_command: usize = s
            .lines()
            .filter(|l| l.contains("xxxx"))
            .map(|l| l.matches('x').count())
            .sum();
        assert_eq!(in_command, 300, "never truncated");
    }

    #[test]
    fn the_input_line_is_offered_only_when_a_command_can_go_there() {
        for risk in [Risk::Low, Risk::High] {
            let with = render(&req("ls", risk), 80, true, &plain());
            assert!(with.contains("prompt"), "{risk:?}: {with}");
            let without = render(
                &ApprovalRequest {
                    can_buffer: false,
                    ..req("ls", risk)
                },
                80,
                true,
                &plain(),
            );
            assert!(
                !without.contains("[b]")
                    && !without.contains("b + Enter")
                    && !without.contains("prompt"),
                "{risk:?}: {without}"
            );
            // what remains is still a prompt that can be answered
            assert!(
                without.contains("[y] run") || without.contains("type yes"),
                "{without}"
            );
        }
    }

    #[test]
    fn high_risk_asks_for_a_word() {
        let hi = render(&req("rm -rf x", Risk::High), 80, true, &plain());
        assert!(hi.contains("type yes"), "{hi}");
        let lo = render(&req("ls", Risk::Low), 80, true, &plain());
        assert!(lo.contains("[y] run") && !lo.contains("type yes"));
        assert!(lo.contains("risk: low") && hi.contains("risk: high"));
        assert!(hi.contains("modifies or destroys"));
    }

    #[test]
    fn ascii_fallback_has_no_box_characters() {
        let s = render(&req("ls", Risk::Low), 80, false, &plain());
        assert!(s.is_ascii(), "{s}");
    }

    /// `s` without its styles (`ESC [ digits and ; m`).
    fn strip(s: &str) -> String {
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

    #[test]
    fn colour_changes_how_the_prompt_looks_and_not_what_it_says() {
        for risk in [Risk::Low, Risk::Medium, Risk::High] {
            let r = req("rm -rf x", risk);
            let styled = render(&r, 80, true, &*colour());
            assert!(styled.contains("\u{1b}["), "{risk:?}");
            assert_eq!(strip(&styled), render(&r, 80, true, &plain()), "{risk:?}");
        }
    }

    #[test]
    fn the_risk_is_green_yellow_or_red_and_the_command_has_no_style() {
        let at = |risk| render(&req("ls", risk), 80, true, &*colour());
        assert!(at(Risk::Low).contains("\u{1b}[1m\u{1b}[32mlow\u{1b}[0m"));
        assert!(at(Risk::Medium).contains("\u{1b}[33mmedium\u{1b}[0m"));
        assert!(at(Risk::High).contains("\u{1b}[1m\u{1b}[31mhigh\u{1b}[0m"));
        // the command is the one line that is read exactly as it will run
        let line = at(Risk::Low)
            .lines()
            .find(|l| l.contains("$ ls"))
            .unwrap()
            .to_string();
        assert!(line.ends_with("$ ls"), "{line:?}");
    }

    #[test]
    fn styling_does_not_change_where_a_long_command_is_cut() {
        let long = "x".repeat(300);
        let r = req(&long, Risk::Low);
        let lines = |s: &str| {
            s.lines()
                .filter(|l| l.contains("xxxx"))
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let plain_out = render(&r, 60, true, &plain());
        let styled = render(&r, 60, true, &*colour());
        assert_eq!(lines(&strip(&styled)), lines(&plain_out));
        assert!(lines(&strip(&styled))
            .iter()
            .all(|l| l.chars().count() <= 60));
    }

    #[test]
    fn hostile_text_in_a_styled_prompt_adds_no_escape_of_its_own() {
        let r = ApprovalRequest {
            reasons: vec!["why \u{1b}[2J".into()],
            cwd: Some("/x\u{1b}]0;t\u{7}".into()),
            ..req("echo \u{1b}[31mred", Risk::High)
        };
        let out = strip(&render(&r, 80, true, &*colour()));
        assert!(!out.contains('\u{1b}') && !out.contains('\u{7}'), "{out:?}");
    }
}
