//! What this process's output may look like: colour and markdown, decided from the config and the
//! environment, once per stream.

use crate::tty::Tty;
use shellmate_core::ports::SystemEnv;
use shellmate_core::present::{self, Caps, ColorSetting, Presenter};
use shellmate_core::Config;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static CONFIG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
static SETTINGS: OnceLock<(ColorSetting, bool)> = OnceLock::new();

/// Remember which config file to read the `[ui]` settings from, when something needs them.
pub(crate) fn init(config: Option<&Path>) {
    let _ = CONFIG_PATH.set(config.map(Path::to_path_buf));
}

/// A command that has already loaded the config shares its `[ui]` settings.
pub(crate) fn use_config(cfg: &Config) {
    let _ = SETTINGS.set((cfg.ui.color, cfg.ui.markdown));
}

/// `[ui] color` and `[ui] markdown`. A config that cannot be read changes nothing: the defaults apply, and
/// the command reports the config problem itself.
fn settings() -> (ColorSetting, bool) {
    *SETTINGS.get_or_init(|| {
        let path = CONFIG_PATH.get().and_then(|p| p.as_deref());
        Config::load(path)
            .map(|c| (c.ui.color, c.ui.markdown))
            .unwrap_or((ColorSetting::Auto, true))
    })
}

/// The caps for the controlling terminal `tty`.
pub(crate) fn tty_caps(tty: &Tty) -> Caps {
    let (color, markdown) = settings();
    tty.caps_with(color, markdown)
}

/// The caps for a standard stream (`1` is stdout, `2` is stderr).
fn stream_caps(fd: i32) -> Caps {
    let (color, markdown) = settings();
    // SAFETY: isatty and ioctl only read from the descriptor.
    let is_terminal = unsafe { libc::isatty(fd) } == 1;
    let width = if is_terminal {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
            usize::from(ws.ws_col)
        } else {
            80
        }
    } else {
        80
    };
    Caps::detect(&SystemEnv, is_terminal, width, color, markdown)
}

/// The caps for what a command prints on stdout.
pub(crate) fn stdout_caps() -> Caps {
    stream_caps(1)
}

/// The presenter for what a command prints on stdout.
pub(crate) fn stdout() -> Box<dyn Presenter> {
    present::presenter(stdout_caps())
}

/// The presenter for what a command prints on stderr.
pub(crate) fn stderr() -> Box<dyn Presenter> {
    present::presenter(stream_caps(2))
}
