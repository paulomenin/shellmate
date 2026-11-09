//! The text-only presenter: word-wrapped, escaped, no colour, no markdown. This is what shellmate has
//! always printed, and the fallback for any terminal that cannot show more.

use super::{Caps, Mark, Notice, Presenter, Remark, Tone};
use crate::render::{wrap, Wrapper};
use crate::sanitize::escape_keep_layout;
use shellmate_api::Error;

pub struct PlainPresenter {
    caps: Caps,
    wrapper: Wrapper,
    wrote: bool,
    /// Some text other than line breaks has been given to the presenter (it may still be held back).
    started: bool,
    at_line_start: bool,
}

/// Erases the waiting indicator (`\r` + clear to end of line).
const ERASE_SPINNER: &str = "\r\x1b[K";

impl PlainPresenter {
    pub fn new(caps: Caps) -> Self {
        Self {
            caps,
            wrapper: Wrapper::new(caps.width),
            wrote: false,
            started: false,
            at_line_start: true,
        }
    }

    /// The first thing written also erases the waiting indicator.
    fn emit(&mut self, out: String) -> String {
        if out.is_empty() {
            return out;
        }
        let mut s = String::new();
        if !self.wrote {
            self.wrote = true;
            s.push_str(ERASE_SPINNER);
        }
        self.at_line_start = out.ends_with('\n');
        s.push_str(&out);
        s
    }
}

impl Presenter for PlainPresenter {
    fn answer(&mut self, text: &str) -> String {
        // with nothing above it, leading blank lines have nothing to separate
        let text = if self.started {
            text
        } else {
            text.trim_start_matches('\n')
        };
        self.started |= !text.is_empty();
        let out = self.wrapper.push(text);
        self.emit(out)
    }

    fn notice(&mut self, notice: &Notice) -> String {
        let gap = if notice.gap { "\n" } else { "" };
        self.answer(&format!("{gap}{}\n", notice.plain(self.caps.unicode)))
    }

    fn finish(&mut self) -> String {
        let tail = self.wrapper.finish();
        let mut out = self.emit(tail);
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
        let text = remarks
            .iter()
            .map(Remark::plain)
            .collect::<Vec<_>>()
            .join("\n\n");
        let gap = if self.wrote { "\n" } else { "" };
        format!("{gap}{}\n", wrap(&text, self.caps.width))
    }

    fn wrote_any(&self) -> bool {
        self.wrote
    }

    fn error(&self, error: &Error) -> String {
        let cross = Mark::Cross.glyph(self.caps.unicode);
        let arrow = if self.caps.unicode { "↳" } else { ">" };
        let mut s = wrap(
            &format!("{cross} {}: {}", error.code, error.message),
            self.caps.width,
        );
        s.push('\n');
        if let Some(hint) = &error.hint {
            s.push_str(&wrap(&format!("{arrow} {hint}"), self.caps.width));
            s.push('\n');
        }
        s
    }

    fn status(&self, _tone: Tone, text: &str) -> String {
        format!("{text}\n")
    }

    fn spinner(&self, step: usize) -> String {
        let frames: &[&str] = if self.caps.unicode {
            &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        } else {
            &["-", "\\", "|", "/"]
        };
        format!("\r{} thinking…", frames[step % frames.len()])
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

    fn label(&self, _tone: Tone, text: &str) -> String {
        escape_keep_layout(text)
    }

    fn cli_error(&self, error: &Error) -> String {
        let mut s = format!(
            "{}: {}\n",
            escape_keep_layout(error.code),
            escape_keep_layout(&error.message)
        );
        if let Some(hint) = &error.hint {
            s.push_str(&format!("  {}\n", escape_keep_layout(hint)));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNICODE: Caps = Caps::plain(40, true);
    const ASCII: Caps = Caps::plain(40, false);

    fn plain(caps: Caps) -> PlainPresenter {
        PlainPresenter::new(caps)
    }

    #[test]
    fn first_output_erases_the_spinner_once() {
        let mut p = plain(UNICODE);
        assert_eq!(p.answer("Hel"), "");
        assert!(!p.wrote_any(), "a partial word is held back");
        assert_eq!(p.answer("lo world "), "\r\x1b[KHello world");
        assert!(p.wrote_any());
        assert_eq!(p.answer("again "), " again");
    }

    #[test]
    fn finish_ends_on_a_fresh_line_exactly_once() {
        let mut p = plain(UNICODE);
        p.answer("one two");
        let end = p.finish();
        assert!(end.ends_with("two\n"), "{end:?}");
        assert_eq!(p.finish(), "");
        let mut q = plain(UNICODE);
        q.answer("line\n");
        assert_eq!(q.finish(), "", "already at a line start");
    }

    #[test]
    fn nothing_written_means_no_output_at_all() {
        let mut p = plain(UNICODE);
        assert_eq!(p.finish(), "");
        assert_eq!(p.remarks(&[]), "");
        assert!(!p.wrote_any());
    }

    #[test]
    fn leading_blank_lines_are_dropped_only_before_the_first_output() {
        let mut p = plain(UNICODE);
        assert_eq!(p.answer("\n\nHello\n"), "\r\x1b[KHello\n");
        assert_eq!(p.answer("\nNext\n"), "\nNext\n");
    }

    #[test]
    fn a_break_right_after_the_first_word_is_kept() {
        // the first word is held back until it is complete; a line break arriving next must not be
        // mistaken for a blank line above the answer
        let mut p = plain(UNICODE);
        assert_eq!(p.answer("Hello"), "");
        let mut out = p.answer("\n\n");
        out += &p.answer("World");
        out += &p.finish();
        assert_eq!(out, "\r\x1b[KHello\n\nWorld\n");
    }

    #[test]
    fn hostile_text_is_escaped() {
        let mut p = plain(UNICODE);
        let out = p.answer("hi \u{1b}[2J there ") + &p.finish();
        assert!(!out.contains("\u{1b}[2J"));
        assert!(out.contains("\\u{1b}[2J"));
    }

    #[test]
    fn a_notice_is_its_symbol_its_text_and_its_detail() {
        let n = Notice::new(Tone::Success, Mark::Check, "ran (approved): ls")
            .with_detail(vec!["one".into(), "two".into()]);
        assert_eq!(n.plain(true), "✔ ran (approved): ls\n  one\n  two");
        assert_eq!(n.plain(false), "+ ran (approved): ls\n  one\n  two");
        // the wrapper, as for any text, keeps the line breaks and drops indentation at a line start
        let mut p = plain(UNICODE);
        assert_eq!(p.notice(&n), "\r\x1b[K✔ ran (approved): ls\none\ntwo\n");
    }

    #[test]
    fn every_mark_has_a_unicode_and_an_ascii_form() {
        let forms = [
            (Mark::Check, "✔", "+"),
            (Mark::Cross, "✘", "x"),
            (Mark::Arrow, "→", "->"),
            (Mark::Chevron, ">", ">"),
        ];
        for (mark, uni, ascii) in forms {
            assert_eq!((mark.glyph(true), mark.glyph(false)), (uni, ascii));
        }
    }

    #[test]
    fn a_notice_after_unfinished_text_starts_on_a_new_line() {
        let mut p = plain(UNICODE);
        // a word is only written once it is complete, so end the text after a space
        assert_eq!(p.answer("Let me look. "), "\r\x1b[KLet me look.");
        let mut n = Notice::new(Tone::Warning, Mark::Chevron, "denied: exec ls");
        n.gap = true;
        assert_eq!(p.notice(&n), "\n> denied: exec ls\n");
        // text that already ended its line needs no gap
        let mut q = plain(UNICODE);
        q.answer("Let me look.\n");
        let n = Notice::new(Tone::Warning, Mark::Chevron, "denied: exec ls");
        assert_eq!(q.notice(&n), "> denied: exec ls\n");
    }

    #[test]
    fn a_notice_with_nothing_above_it_does_not_start_with_a_blank_line() {
        let mut p = plain(UNICODE);
        let mut n = Notice::new(Tone::Info, Mark::Arrow, "moved to your prompt: ls");
        n.gap = true;
        assert_eq!(p.notice(&n), "\r\x1b[K→ moved to your prompt: ls\n");
    }

    #[test]
    fn remarks_are_paragraphs_labelled_and_wrapped() {
        let mut p = plain(UNICODE);
        p.answer("Answer.\n");
        let r = [
            Remark::new(Tone::Info, "Other options:\nls -a"),
            Remark::labelled(
                Tone::Warning,
                "check",
                "`x` was not found in PATH (it may be an alias)",
            ),
        ];
        assert_eq!(
            p.remarks(&r),
            "\nOther options:\nls -a\n\ncheck: `x` was not found in PATH (it may\nbe an alias)\n"
        );
        // with no answer above them there is no blank line first
        let mut q = plain(UNICODE);
        assert_eq!(q.remarks(&r[..1]), "Other options:\nls -a\n");
    }

    #[test]
    fn an_error_shows_its_code_message_and_hint() {
        let e = Error::new("provider.auth", "HTTP 401").with_hint("check the key");
        assert_eq!(
            plain(UNICODE).error(&e),
            "✘ provider.auth: HTTP 401\n↳ check the key\n"
        );
        assert_eq!(
            plain(ASCII).error(&e),
            "x provider.auth: HTTP 401\n> check the key\n"
        );
        let bare = Error::new("glue.protocol", "unknown action");
        assert_eq!(
            plain(UNICODE).error(&bare),
            "✘ glue.protocol: unknown action\n"
        );
    }

    #[test]
    fn a_long_error_is_wrapped_to_the_width() {
        let e = Error::new("x.y", "word ".repeat(30));
        assert!(plain(UNICODE)
            .error(&e)
            .lines()
            .all(|l| l.chars().count() <= 40));
    }

    #[test]
    fn status_lines_are_plain() {
        assert_eq!(
            plain(UNICODE).status(Tone::Muted, "[cancelled]"),
            "[cancelled]\n"
        );
    }

    #[test]
    fn the_spinner_cycles_through_its_frames_and_clears_in_place() {
        let p = plain(UNICODE);
        assert_eq!(p.spinner(0), "\r⠋ thinking…");
        assert_eq!(p.spinner(10), p.spinner(0));
        assert_eq!(plain(ASCII).spinner(1), "\r\\ thinking…");
        assert_eq!(p.clear_spinner(), "\r\x1b[K");
    }

    #[test]
    fn the_bullet_follows_the_terminal() {
        assert_eq!(plain(UNICODE).bullet(), "•");
        assert_eq!(plain(ASCII).bullet(), "*");
    }
}
