//! `render`: show markdown the way an answer would be shown, to see the effect of your terminal and settings.

use super::ui;
use shellmate_api::Error;
use shellmate_core::ports::SystemEnv;
use shellmate_core::present::{self, Caps, Level};
use std::io::Read;
use std::path::Path;

pub(crate) fn run(
    file: Option<&Path>,
    color: Option<&str>,
    width: Option<usize>,
    ascii: bool,
    no_markdown: bool,
) -> i32 {
    super::finish_text(show(file, color, width, ascii, no_markdown))
}

fn show(
    file: Option<&Path>,
    color: Option<&str>,
    width: Option<usize>,
    ascii: bool,
    no_markdown: bool,
) -> Result<String, Error> {
    let text = match file {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| Error::new("render.unreadable", format!("{}: {e}", path.display())))?,
        None => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| Error::new("render.unreadable", format!("reading input: {e}")))?;
            s
        }
    };
    let detected = ui::stdout_caps();
    let level = match color {
        None | Some("auto") => detected.level,
        Some("plain") => Level::Plain,
        Some("mono") => Level::Mono,
        _ => Level::Ansi16,
    };
    let caps = Caps {
        width: width.unwrap_or(detected.width),
        unicode: !ascii && shellmate_core::present::locale_is_utf8(&SystemEnv),
        level,
        // markdown is formatted at every level above plain, unless you ask for the text as written
        markdown: !no_markdown && level > Level::Plain,
    };
    let mut p = present::presenter(caps);
    let mut out = p.answer(&text);
    out.push_str(&p.finish());
    // the first output of an answer erases a waiting indicator; there is none here
    Ok(out.strip_prefix("\r\x1b[K").unwrap_or(&out).to_string())
}
