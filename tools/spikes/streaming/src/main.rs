//! Spike: stream an OpenAI-style SSE response to /dev/tty, append-only, word-wrapped,
//! with spinner, Ctrl-C/Esc cancel, sanitized output, latency measurements.
//! usage: streaming-spike <url> ; exit 0 done, 10 cancelled/unchanged, 1 error. stdout stays empty.
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Tty { fd: i32, saved: libc::termios }
impl Tty {
    fn open() -> Option<Tty> {
        let fd = unsafe { libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 { return None; }
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut t) } != 0 { return None; }
        let saved = t;
        if std::env::var("SPIKE_MODE").as_deref() == Ok("sigint") {
            // keep canonical mode + ISIG: typeahead stays queued in the kernel for the shell; only hide echo
            t.c_lflag &= !libc::ECHO;
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) };
        } else {
            t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
            t.c_cc[libc::VMIN] = 1; t.c_cc[libc::VTIME] = 0;
            unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &t) };
        }
        Some(Tty { fd, saved })
    }
    fn write(&self, s: &str) {
        let b = s.as_bytes(); let mut off = 0;
        while off < b.len() {
            let n = unsafe { libc::write(self.fd, b[off..].as_ptr().cast(), b.len() - off) };
            if n <= 0 { break; } off += n as usize;
        }
    }
    fn width(&self) -> usize {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(self.fd, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 { ws.ws_col as usize } else { 80 }
    }
}
impl Drop for Tty { fn drop(&mut self) { unsafe { libc::tcsetattr(self.fd, libc::TCSADRAIN, &self.saved); libc::close(self.fd); } } }

/// Append-only renderer: buffers one word so wrapping never splits a word (long words are hard-split).
struct Render { width: usize, col: usize, word: String, wrote_any: bool }
impl Render {
    fn push(&mut self, t: &Tty, text: &str, out: &mut String) {
        for c in text.chars() {
            match c {
                '\n' => { self.flush_word(out); out.push_str("\r\n"); self.col = 0; }
                ' ' | '\t' => { self.flush_word(out); if self.col + 1 >= self.width { out.push_str("\r\n"); self.col = 0 } else if self.col > 0 { out.push(' '); self.col += 1; } }
                c if c.is_control() => self.word.push_str(&format!("\\u{{{:x}}}", c as u32)),
                '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}' => self.word.push_str(&format!("\\u{{{:x}}}", c as u32)),
                c => self.word.push(c),
            }
        }
        let _ = t;
    }
    fn flush_word(&mut self, out: &mut String) {
        if self.word.is_empty() { return; }
        let w: Vec<char> = self.word.chars().collect();
        let mut i = 0;
        while i < w.len() {
            let room = self.width.saturating_sub(self.col + 1);
            if room == 0 || (w.len() - i > room && self.col > 0 && w.len() - i <= self.width - 1) { out.push_str("\r\n"); self.col = 0; continue; }
            let take = room.min(w.len() - i);
            out.extend(&w[i..i + take]); self.col += take; i += take;
            if i < w.len() { out.push_str("\r\n"); self.col = 0; }
        }
        self.word.clear();
    }
}

fn parse_event(ev: &str) -> Option<Result<String, ()>> {
    let mut data = String::new();
    for l in ev.lines() {
        if let Some(d) = l.strip_prefix("data:") { data.push_str(d.strip_prefix(' ').unwrap_or(d)); }
    }
    if data.is_empty() { return None; }
    if data == "[DONE]" { return Some(Err(())); }
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    v["choices"][0]["delta"]["content"].as_str().map(|s| Ok(s.to_string()))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let url = std::env::args().nth(1).expect("url");
    let Some(tty) = Tty::open() else { eprintln!("no tty"); std::process::exit(1) };
    let cancel = Arc::new(AtomicBool::new(false));
    static SIG: AtomicBool = AtomicBool::new(false);
    extern "C" fn on_sig(_: libc::c_int) { SIG.store(true, Ordering::SeqCst); }
    let sigmode = std::env::var("SPIKE_MODE").as_deref() == Ok("sigint");
    if sigmode { unsafe { let mut sa: libc::sigaction = std::mem::zeroed(); sa.sa_sigaction = on_sig as usize; libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut()); libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut()); } }
    if !sigmode { // key watcher: any of Ctrl-C / Esc / q cancels
        let (fd, c) = (tty.fd, cancel.clone());
        std::thread::spawn(move || loop {
            let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            if unsafe { libc::poll(&mut p, 1, 100) } > 0 {
                let mut b = [0u8; 16]; let n = unsafe { libc::read(fd, b.as_mut_ptr().cast(), 16) };
                if n == 1 && matches!(b[0], 0x03 | 0x1b | b'q') { c.store(true, Ordering::SeqCst); }
            }
            if c.load(Ordering::SeqCst) { return; }
        });
    }
    let t0 = Instant::now();
    let mut r = Render { width: tty.width(), col: 0, word: String::new(), wrote_any: false };
    let client = reqwest::Client::new();
    let mut resp = match client.post(&url).body("{}").send().await { Ok(r) => r, Err(e) => { tty.write(&format!("\r\nerror: {e}\r\n")); std::process::exit(1) } };
    let mut raw: Vec<u8> = Vec::new();
    let (mut first_event, mut first_write): (Option<Duration>, Option<Duration>) = (None, None);
    let mut lat: Vec<u128> = Vec::new();
    let mut frame = 0usize;
    let mut tick = tokio::time::interval(Duration::from_millis(80));
    let mut done = false; let mut cancelled = false;
    // spinner state: drawn on its own line until the first token
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    tty.write("\r\n");
    while !done {
        tokio::select! {
            _ = tick.tick() => {
                if cancel.load(Ordering::SeqCst) || SIG.load(Ordering::SeqCst) { cancelled = true; break; }
                if !r.wrote_any { tty.write(&format!("\r{} thinking…", spinner[frame % 10])); frame += 1; }
            }
            chunk = resp.chunk() => {
                let Ok(Some(b)) = chunk else { break };
                let recv = Instant::now();
                raw.extend_from_slice(&b);
                let mut out = String::new();
                loop { // events end with a blank line (LF or CRLF)
                    let pos = raw.windows(2).position(|w| w == b"\n\n").map(|p| (p, 2)).or_else(|| raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| (p, 4)));
                    let Some((p, l)) = pos else { break };
                    let evb: Vec<u8> = raw.drain(..p + l).collect();
                    let ev = String::from_utf8_lossy(&evb[..p]).into_owned(); // complete events only: never splits a UTF-8 char
                    match parse_event(&ev) {
                        Some(Ok(text)) => { if first_event.is_none() { first_event = Some(t0.elapsed()); } r.push(&tty, &text, &mut out); }
                        Some(Err(())) => { done = true; }
                        None => {}
                    }
                }
                if !out.is_empty() {
                    if !r.wrote_any { tty.write("\r\x1b[K"); r.wrote_any = true; } // erase spinner line
                    tty.write(&out);
                    if first_write.is_none() { first_write = Some(t0.elapsed()); }
                    lat.push(recv.elapsed().as_micros());
                }
                if cancel.load(Ordering::SeqCst) || SIG.load(Ordering::SeqCst) { cancelled = true; break; }
            }
        }
    }
    let mut tail = String::new(); r.flush_word(&mut tail);
    if !r.wrote_any { tty.write("\r\x1b[K"); }
    tty.write(&tail);
    tty.write(if cancelled { "\r\n[cancelled]\r\n" } else { "\r\n" });
    drop(resp); // closes the connection => provider stops generating
    lat.sort();
    let p = |q: f64| lat.get(((lat.len() as f64 * q) as usize).min(lat.len().saturating_sub(1))).copied().unwrap_or(0);
    let mut e = std::io::stderr();
    let _ = writeln!(e, "first_event={:?} first_write={:?} writes={} recv→write p50={}µs p95={}µs max={}µs total={:?} cancelled={}", first_event, first_write, lat.len(), p(0.5), p(0.95), p(1.0), t0.elapsed(), cancelled);
    drop(tty);
    std::process::exit(if cancelled { 10 } else { 0 });
}
