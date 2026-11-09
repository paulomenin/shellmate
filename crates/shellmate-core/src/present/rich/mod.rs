//! The presenter for terminals that can show styling: colour or just bold and italic, and the model's
//! markdown formatted.
//!
//! The text it shows is the text the plain presenter shows, with the markdown markers of the answer turned
//! into looks. The only escape sequences it writes are the theme's styles and the line erase of the waiting
//! indicator; everything else that reaches the terminal goes through the wrapper, which escapes it.

mod markdown;
mod wrap;

use super::{Caps, Mark, Notice, Presenter, Remark, Role, Theme, Tone};
use crate::sanitize::escape_keep_layout;
use markdown::Markdown;
use shellmate_api::Error;
use wrap::Wrap;

pub struct RichPresenter {
    caps: Caps,
    theme: Theme,
    wrap: Wrap,
    markdown: Option<Markdown>,
    wrote: bool,
    /// Some text other than line breaks has been given to the presenter (it may still be held back).
    started: bool,
    at_line_start: bool,
}

/// Erases the waiting indicator (`\r` + clear to end of line).
const ERASE_SPINNER: &str = "\r\x1b[K";

impl RichPresenter {
    pub fn new(caps: Caps) -> Self {
        let theme = Theme::new(caps.level);
        Self {
            caps,
            theme,
            wrap: Wrap::new(caps.width, theme),
            markdown: caps
                .markdown
                .then(|| Markdown::new(caps.width, caps.unicode)),
            wrote: false,
            started: false,
            at_line_start: true,
        }
    }

    /// What the wrapper has ready. The first thing written also erases the waiting indicator.
    fn emit(&mut self) -> String {
        let out = self.wrap.take();
        if out.is_empty() {
            return out;
        }
        self.at_line_start = out.ends_with('\n');
        if self.wrote {
            return out;
        }
        self.wrote = true;
        format!("{ERASE_SPINNER}{out}")
    }

    /// End the line being read, so that what comes next starts on a line of its own.
    fn interrupt(&mut self) {
        match &mut self.markdown {
            Some(md) => md.interrupt(&mut self.wrap),
            None => {
                self.wrap.finish();
                if !self.wrap.at_line_start() {
                    self.wrap.newline();
                }
            }
        }
    }

    fn role(tone: Tone) -> Role {
        Role::from(tone)
    }

    fn fresh(&self) -> Wrap {
        Wrap::new(self.caps.width, self.theme)
    }
}

impl Presenter for RichPresenter {
    fn answer(&mut self, text: &str) -> String {
        // with nothing above it, leading blank lines have nothing to separate
        let text = if self.started {
            text
        } else {
            text.trim_start_matches('\n')
        };
        self.started |= !text.is_empty();
        match &mut self.markdown {
            Some(md) => md.push(text, &mut self.wrap),
            None => self.wrap.text(text, Role::Plain),
        }
        self.emit()
    }

    fn notice(&mut self, notice: &Notice) -> String {
        self.interrupt();
        let glyph = format!("{} ", notice.mark.glyph(self.caps.unicode));
        let w = &mut self.wrap;
        w.block((&glyph, Self::role(notice.tone)), ("  ", Role::Plain));
        w.text(&notice.text, Role::Plain);
        if let Some(meta) = &notice.meta {
            w.text(" ", Role::Plain);
            w.text(&format!("[{meta}]"), Role::Muted);
        }
        w.newline();
        for line in &notice.detail {
            w.text(line, Role::Muted);
            w.newline();
        }
        w.end_block();
        // a notice is a line of its own, so it also ends what was started before it
        self.started = true;
        self.emit()
    }

    fn finish(&mut self) -> String {
        match &mut self.markdown {
            Some(md) => md.finish(&mut self.wrap),
            None => self.wrap.finish(),
        }
        let mut out = self.emit();
        if self.wrote && !self.at_line_start {
            out.push('\n');
            self.at_line_start = true;
        }
        out
    }

    fn remarks(&mut self, remarks: &[Remark]) -> String {
        if remarks.is_empty() {
            return String::new();
        }
        let mut w = self.fresh();
        for (i, remark) in remarks.iter().enumerate() {
            if i > 0 {
                w.newline();
            }
            let role = Self::role(remark.tone);
            match remark.label {
                Some(label) => {
                    w.text(&format!("{label}:"), role);
                    w.text(" ", Role::Plain);
                    w.text(&remark.text, Role::Plain);
                }
                None => w.text(&remark.text, role),
            }
            w.newline();
            if !remark.items.is_empty() {
                w.block(("  ", Role::Plain), ("  ", Role::Plain));
                for item in &remark.items {
                    w.text(item, Role::Code);
                    w.newline();
                }
                w.end_block();
            }
        }
        w.finish();
        let gap = if self.wrote { "\n" } else { "" };
        format!("{gap}{}", w.take())
    }

    fn wrote_any(&self) -> bool {
        self.wrote
    }

    fn error(&self, error: &Error) -> String {
        let mut w = self.fresh();
        let cross = format!("{} ", Mark::Cross.glyph(self.caps.unicode));
        w.block((&cross, Role::Error), ("  ", Role::Plain));
        w.text(&format!("{}:", error.code), Role::Error);
        w.text(" ", Role::Plain);
        w.text(&error.message, Role::Plain);
        w.newline();
        w.end_block();
        if let Some(hint) = &error.hint {
            let arrow = if self.caps.unicode { "↳ " } else { "> " };
            w.block((arrow, Role::Info), ("  ", Role::Plain));
            w.text(hint, Role::Plain);
            w.newline();
            w.end_block();
        }
        w.finish();
        w.take()
    }

    fn status(&self, tone: Tone, text: &str) -> String {
        format!(
            "{}\n",
            self.theme
                .paint(Self::role(tone), &escape_keep_layout(text))
        )
    }

    fn spinner(&self, step: usize) -> String {
        let frames: &[&str] = if self.caps.unicode {
            &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        } else {
            &["-", "\\", "|", "/"]
        };
        let style = self.theme.spinner(step);
        format!(
            "\r{style}{}{style:#} {}",
            frames[step % frames.len()],
            self.theme.paint(Role::Muted, "thinking…")
        )
    }

    fn clear_spinner(&self) -> String {
        ERASE_SPINNER.to_string()
    }

    fn bullet(&self) -> &'static str {
        if self.caps.unicode {
            "•"
        } else {
            "*"
        }
    }

    fn label(&self, tone: Tone, text: &str) -> String {
        self.theme
            .paint(Self::role(tone), &escape_keep_layout(text))
    }

    fn cli_error(&self, error: &Error) -> String {
        let mut s = format!(
            "{}: {}\n",
            self.theme
                .paint(Role::Error, &escape_keep_layout(error.code)),
            escape_keep_layout(&error.message)
        );
        if let Some(hint) = &error.hint {
            s.push_str(&format!(
                "  {}\n",
                self.theme.paint(Role::Muted, &escape_keep_layout(hint))
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests;
