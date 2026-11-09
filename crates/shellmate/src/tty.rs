//! The client's terminal: `/dev/tty` (never stdout, which carries the new buffer).
//!
//! While waiting, the tty stays in canonical mode with only `ECHO` off and `ISIG` on, so Ctrl-C reaches
//! us as SIGINT and typeahead stays queued for the shell; the saved termios is restored with
//! `TCSADRAIN` on every exit path (`spike-streaming.md`).

use std::io::Write;

pub struct Tty {
    fd: i32,
    saved: Option<libc::termios>,
    /// Fallback sink when there is no controlling terminal (tests, pipes).
    fallback: bool,
    utf8: bool,
}

impl Tty {
    pub fn open() -> Self {
        let utf8 = shellmate_core::present::locale_is_utf8(&shellmate_core::ports::SystemEnv);
        let fd = unsafe {
            libc::open(
                c"/dev/tty".as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOCTTY,
            )
        };
        if fd < 0 {
            return Self {
                fd: -1,
                saved: None,
                fallback: true,
                utf8,
            };
        }
        let mut t: libc::termios = unsafe { std::mem::zeroed() };
        let saved = if unsafe { libc::tcgetattr(fd, &mut t) } == 0 {
            let original = t;
            t.c_lflag &= !libc::ECHO;
            t.c_lflag |= libc::ISIG;
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) };
            Some(original)
        } else {
            None
        };
        Self {
            fd,
            saved,
            fallback: false,
            utf8,
        }
    }

    /// False when there is no controlling terminal (output goes to stderr; no spinner).
    pub fn is_tty(&self) -> bool {
        !self.fallback
    }

    pub fn utf8(&self) -> bool {
        self.utf8
    }

    /// What a presenter needs to know to draw for this terminal, from the environment and the user's
    /// `[ui]` settings.
    pub fn caps_with(
        &self,
        color: shellmate_core::present::ColorSetting,
        markdown: bool,
    ) -> shellmate_core::present::Caps {
        shellmate_core::present::Caps::detect(
            &shellmate_core::ports::SystemEnv,
            self.is_tty(),
            self.width(),
            color,
            markdown,
        )
    }

    pub fn width(&self) -> usize {
        if self.fallback {
            return 80;
        }
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(self.fd, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
            usize::from(ws.ws_col)
        } else {
            80
        }
    }

    /// Throw away everything typed while we were waiting. Called once the request has finished, whatever the outcome:
    /// an Enter pressed during the wait is still queued, and the shell would read it right after the widget
    /// returns and run the suggestion before the user has seen it. Returns true if a completed line (an Enter) was discarded (in canonical mode only those are counted; partial typing is discarded too),
    /// so the caller can say so.
    pub fn discard_pending_input(&self) -> bool {
        if self.fallback {
            return false;
        }
        let mut queued: libc::c_int = 0;
        let had_line =
            unsafe { libc::ioctl(self.fd, libc::FIONREAD, &mut queued) } == 0 && queued > 0;
        unsafe { libc::tcflush(self.fd, libc::TCIFLUSH) };
        had_line
    }

    pub fn write(&self, s: &str) {
        if self.fallback {
            let mut e = std::io::stderr().lock();
            let _ = e.write_all(s.as_bytes());
            let _ = e.flush();
            return;
        }
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
}

impl Drop for Tty {
    fn drop(&mut self) {
        if self.fd >= 0 {
            if let Some(t) = &self.saved {
                unsafe { libc::tcsetattr(self.fd, libc::TCSADRAIN, t) };
            }
            unsafe { libc::close(self.fd) };
        }
    }
}
