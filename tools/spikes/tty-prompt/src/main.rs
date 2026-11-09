//! Phase 0 spike: inline approval prompt on /dev/tty.
//! usage: tty-prompt-spike [--risk high] [--panic] [--hold] -- <command text (untrusted)>
//! exit: 0 allow, 1 deny, 2 no tty (deny)
use std::sync::atomic::{AtomicBool, Ordering};

static SIGNALED: AtomicBool = AtomicBool::new(false);
extern "C" fn on_signal(_: libc::c_int) {
    SIGNALED.store(true, Ordering::SeqCst);
}

struct RawTty {
    fd: i32,
    saved: libc::termios,
}
impl RawTty {
    fn open() -> Option<RawTty> {
        let fd = unsafe { libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
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
        // TCSAFLUSH also discards pending input; we flush explicitly again just before drawing.
        unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &t) };
        Some(RawTty { fd, saved })
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
            ws.ws_col as usize
        } else {
            80
        }
    }
    fn flush_input(&self) {
        unsafe { libc::tcflush(self.fd, libc::TCIFLUSH) };
    }
    /// Wait up to `ms` for input. Returns false on timeout / signal.
    fn wait(&self, ms: i32) -> bool {
        let mut p = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
        unsafe { libc::poll(&mut p, 1, ms) > 0 && (p.revents & (libc::POLLIN | libc::POLLHUP)) != 0 }
    }
    fn read(&self, buf: &mut [u8]) -> isize {
        unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) }
    }
}
impl Drop for RawTty {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.saved);
            libc::close(self.fd);
        }
    }
}

/// Make untrusted text safe to display: no control chars, escape sequences, bidi
/// overrides or zero-width characters can reach the terminal; they are shown visibly.
fn sanitize(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\\' => o.push_str("\\\\"),
            c if c.is_control() => o.push_str(&format!("\\u{{{:x}}}", c as u32)),
            '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}' | '\u{feff}' => o.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn wrap(s: &str, width: usize, indent: &str) -> String {
    let w = width.saturating_sub(indent.len() + 1).max(10);
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(w).map(|c| format!("{indent}{}", c.iter().collect::<String>())).collect::<Vec<_>>().join("\r\n")
}

enum Key { Char(u8), Enter, Esc, Eof, Ignored, Signal }

fn read_key(t: &RawTty) -> Key {
    loop {
        if SIGNALED.load(Ordering::SeqCst) { return Key::Signal; }
        if !t.wait(100) { continue; }
        let mut b = [0u8; 64];
        let n = t.read(&mut b);
        if n == 0 { return Key::Eof; }
        if n < 0 { continue; }
        let b = &b[..n as usize];
        return match b {
            [b'\r'] | [b'\n'] => Key::Enter,
            [0x03] | [0x04] => Key::Eof,           // Ctrl-C / Ctrl-D => deny
            [0x1b] => {
                // lone ESC vs start of a sequence: wait briefly for more
                if t.wait(40) { let mut r = [0u8; 64]; t.read(&mut r); Key::Ignored } else { Key::Esc }
            }
            [c] => Key::Char(*c),
            _ => Key::Ignored,                      // multi-byte burst = escape sequence or paste: ignore
        };
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let high = args.windows(2).any(|w| w[0] == "--risk" && w[1] == "high");
    let do_panic = args.iter().any(|a| a == "--panic");
    let hold = args.iter().any(|a| a == "--hold");
    let cmd = args.iter().position(|a| a == "--").map(|i| args[i + 1..].join(" ")).unwrap_or_default();

    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as usize;
        for s in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            libc::sigaction(s, &sa, std::ptr::null_mut());
        }
    }

    let Some(t) = RawTty::open() else {
        eprintln!("shellmate: no tty, denied");
        std::process::exit(2);
    };
    let w = t.width();
    t.flush_input(); // keystrokes typed before the prompt appeared must never answer it
    t.write("\r\n");
    t.write(&format!("┌ run this command?\r\n{}\r\n", wrap(&sanitize(&cmd), w, "│ ")));
    if high {
        t.write("│ HIGH RISK. Type 'yes' + Enter to allow, anything else denies: ");
    } else {
        t.write("└ [y] allow  [n] deny (Enter/Esc/Ctrl-C = deny): ");
    }
    if do_panic { panic!("simulated panic while raw"); }
    if hold { while !SIGNALED.load(Ordering::SeqCst) { t.wait(50); } }

    let allowed = if high {
        let mut typed = String::new();
        loop {
            match read_key(&t) {
                Key::Char(c) if c.is_ascii_graphic() && typed.len() < 8 => { typed.push(c as char); t.write(&(c as char).to_string()); }
                Key::Char(0x7f) => { typed.pop(); }
                Key::Enter => break typed == "yes",
                Key::Ignored => {}
                _ => break false,
            }
        }
    } else {
        loop {
            match read_key(&t) {
                Key::Char(b'y') | Key::Char(b'Y') => break true,
                Key::Ignored => continue,
                _ => break false, // everything else, including unknown keys: deny
            }
        }
    };
    t.write(if allowed { "\r\n✔ allowed\r\n" } else { "\r\n✘ denied\r\n" });
    drop(t);
    std::process::exit(if allowed { 0 } else { 1 });
}
