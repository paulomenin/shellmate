//! Append-only word wrapping for terminal output (`spike-streaming.md`). Pure and tty-free so it
//! is unit-testable; the same type will serve the streaming path (push deltas, then `finish`).

use crate::sanitize::escape_char;

pub struct Wrapper {
    width: usize,
    col: usize,
    word: String,
    word_len: usize,
    pending_space: bool,
}

impl Wrapper {
    pub fn new(width: usize) -> Self {
        Self {
            width: width.max(10),
            col: 0,
            word: String::new(),
            word_len: 0,
            pending_space: false,
        }
    }

    /// Feed text (a whole answer or a streamed delta); returns the text to write now.
    /// Display width is approximated by `char` count (TODO: `unicode-width`).
    pub fn push(&mut self, text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            match c {
                '\n' => {
                    self.flush_word(&mut out);
                    out.push('\n');
                    self.col = 0;
                    self.pending_space = false;
                }
                ' ' | '\t' => {
                    self.flush_word(&mut out);
                    self.pending_space = self.col > 0;
                }
                c => match escape_char(c) {
                    Some(e) => {
                        self.word_len += e.chars().count();
                        self.word.push_str(&e);
                    }
                    None => {
                        self.word_len += 1;
                        self.word.push(c);
                    }
                },
            }
        }
        out
    }

    /// Flush the last word (call once at the end of the text).
    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        self.flush_word(&mut out);
        out
    }

    fn flush_word(&mut self, out: &mut String) {
        if self.word.is_empty() {
            return;
        }
        let word = std::mem::take(&mut self.word);
        let len = std::mem::take(&mut self.word_len);
        let space = usize::from(self.pending_space);
        if self.col > 0 && self.col + space + len > self.width {
            out.push('\n');
            self.col = 0;
        } else if self.col > 0 && self.pending_space {
            out.push(' ');
            self.col += 1;
        }
        self.pending_space = false;
        if len <= self.width {
            out.push_str(&word);
            self.col += len;
        } else {
            // longer than a whole line: hard split
            for ch in word.chars() {
                if self.col >= self.width {
                    out.push('\n');
                    self.col = 0;
                }
                out.push(ch);
                self.col += 1;
            }
        }
    }
}

/// Convenience for non-streaming output.
pub fn wrap(text: &str, width: usize) -> String {
    let mut w = Wrapper::new(width);
    let mut out = w.push(text);
    out.push_str(&w.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_on_word_boundaries() {
        assert_eq!(wrap("aaa bbb ccc ddd", 10), "aaa bbb\nccc ddd");
    }

    #[test]
    fn keeps_paragraphs_and_hard_splits_long_words() {
        assert_eq!(wrap("one\n\ntwo", 20), "one\n\ntwo");
        assert_eq!(
            wrap("abcdefghijklmnopqrstuvwxyz", 10),
            "abcdefghij\nklmnopqrst\nuvwxyz"
        );
    }

    #[test]
    fn streaming_deltas_equal_one_shot() {
        let text = "To list the largest files, use du with sort and a very_long_identifier_that_exceeds_the_width.\nNext line";
        let mut w = Wrapper::new(24);
        let mut streamed = String::new();
        for chunk in text.as_bytes().chunks(5) {
            streamed.push_str(&w.push(std::str::from_utf8(chunk).unwrap_or("")));
            // ASCII only here
        }
        streamed.push_str(&w.finish());
        assert_eq!(streamed, wrap(text, 24));
    }

    #[test]
    fn escapes_control_sequences() {
        let out = wrap("hi \u{1b}[2J there", 40);
        assert!(!out.contains('\u{1b}'));
        assert!(out.contains("\\u{1b}[2J"));
    }

    #[test]
    fn indentation_at_the_start_of_a_line_is_dropped() {
        assert_eq!(wrap("   hi", 20), "hi");
        assert_eq!(wrap("a\n  b", 20), "a\nb");
    }

    #[test]
    fn a_word_that_exactly_fills_the_line_does_not_wrap() {
        // 4 + 1 + 5 = 10 columns
        assert_eq!(wrap("aaaa bbbbb", 10), "aaaa bbbbb");
        assert_eq!(wrap("aaaa bbbbbc", 10), "aaaa\nbbbbbc");
    }

    #[test]
    fn an_escaped_character_counts_with_the_width_it_is_shown_at() {
        // each `\u{1}` is five columns on screen: 4 + 1 + 10 does not fit in 10, but 10 fits a line alone
        assert_eq!(wrap("aaaa \u{1}\u{1}", 10), "aaaa\n\\u{1}\\u{1}");
    }
}
