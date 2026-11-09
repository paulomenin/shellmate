//! Word wrapping over styled text.
//!
//! The same rules as `render::Wrapper` (append-only, a word is written once it is complete, text that is
//! not ours is escaped character by character), plus: runs of text carry a [`Role`], every line of a block
//! can start with a prefix (a list indent, a quote bar), and widths count visible columns, never the escape
//! sequences of the theme.
//!
//! Output depends only on the text, never on how it was split into pieces: nothing is written until a word,
//! a line or the end is complete.

use crate::present::{Role, Theme};
use crate::sanitize::escape_char;

#[derive(Clone, Default)]
struct Prefix {
    /// Safe to show: written by us, not taken from the model.
    text: String,
    role: Option<Role>,
    width: usize,
}

pub(super) struct Wrap {
    width: usize,
    theme: Theme,
    out: String,
    /// Visible columns used on the current line, prefix included.
    col: usize,
    /// Nothing, not even the prefix, has been written on the current line yet.
    at_line_start: bool,
    first: Prefix,
    rest: Prefix,
    /// The next line that gets text is the first line of its block.
    first_line: bool,
    word: Vec<(Role, String)>,
    word_len: usize,
    pending_space: bool,
    /// Styled text that may still grow: the next word in the same style (and the space before it) joins it,
    /// so a span of several words is one run, and an underline has no gaps. Written when the style
    /// changes, at the end of a line and at the end.
    run: Option<(Role, String)>,
}

impl Wrap {
    pub(super) fn new(width: usize, theme: Theme) -> Self {
        Self {
            width: width.max(10),
            theme,
            out: String::new(),
            col: 0,
            at_line_start: true,
            first: Prefix::default(),
            rest: Prefix::default(),
            first_line: true,
            word: Vec::new(),
            word_len: 0,
            pending_space: false,
            run: None,
        }
    }

    /// Start a block: its first line begins with `first`, later lines with `rest`. Both are safe text.
    pub(super) fn block(&mut self, first: (&str, Role), rest: (&str, Role)) {
        // a prefix of more than half the line would leave no room for text: go without it
        let limit = self.width / 2;
        let make = |(text, role): (&str, Role)| {
            let width = text.chars().count();
            if width > limit {
                return Prefix::default();
            }
            Prefix {
                text: text.to_string(),
                role: Some(role),
                width,
            }
        };
        self.first = make(first);
        self.rest = make(rest);
        self.first_line = true;
    }

    /// End the block: lines start without a prefix again.
    pub(super) fn end_block(&mut self) {
        self.first = Prefix::default();
        self.rest = Prefix::default();
        self.first_line = true;
    }

    /// Feed text. `\n` ends a line, spaces separate words, everything else is part of a word.
    pub(super) fn text(&mut self, s: &str, role: Role) {
        for c in s.chars() {
            match c {
                '\n' => {
                    self.flush_word();
                    self.newline();
                }
                ' ' | '\t' => {
                    self.flush_word();
                    self.pending_space = !self.at_line_start;
                }
                c => {
                    let (shown, len) = match escape_char(c) {
                        Some(e) => {
                            let n = e.chars().count();
                            (e, n)
                        }
                        None => (c.to_string(), 1),
                    };
                    self.word_len += len;
                    match self.word.last_mut() {
                        Some((r, t)) if *r == role => t.push_str(&shown),
                        _ => self.word.push((role, shown)),
                    }
                }
            }
        }
    }

    /// Feed preformatted text: spaces are kept and long lines are cut at the width, never re-flowed.
    pub(super) fn raw(&mut self, s: &str, role: Role) {
        self.flush_word();
        for c in s.chars() {
            match c {
                '\n' => self.newline(),
                c => {
                    let shown = match c {
                        '\t' => "    ".to_string(),
                        c => escape_char(c).unwrap_or_else(|| c.to_string()),
                    };
                    for ch in shown.chars() {
                        if !self.at_line_start && self.col >= self.width {
                            self.break_line();
                        }
                        self.start_line();
                        self.put(role, &ch.to_string());
                        self.col += 1;
                    }
                }
            }
        }
    }

    /// Make sure the current line has been started: its prefix is written even if no text follows. For a
    /// list item or a quote with nothing after the marker.
    pub(super) fn open_line(&mut self) {
        self.flush_word();
        self.start_line();
    }

    /// End the current line. An empty line is just a line break, without the prefix.
    pub(super) fn newline(&mut self) {
        self.flush_word();
        self.flush_run();
        self.out.push('\n');
        self.col = 0;
        self.at_line_start = true;
        self.pending_space = false;
    }

    /// Write the last word and line, so everything given so far is in the output.
    pub(super) fn finish(&mut self) {
        self.flush_word();
        self.flush_run();
    }

    /// What is ready to be written. Text still waiting for the end of its word or line stays.
    pub(super) fn take(&mut self) -> String {
        std::mem::take(&mut self.out)
    }

    /// Whether the line being built has nothing on it yet.
    pub(super) fn at_line_start(&self) -> bool {
        self.at_line_start
    }

    fn start_line(&mut self) {
        if !self.at_line_start {
            return;
        }
        self.at_line_start = false;
        let prefix = if self.first_line {
            &self.first
        } else {
            &self.rest
        };
        self.first_line = false;
        if let Some(role) = prefix.role {
            if !prefix.text.is_empty() {
                self.out.push_str(&self.theme.paint(role, &prefix.text));
            }
        }
        self.col = prefix.width;
    }

    fn break_line(&mut self) {
        self.flush_run();
        self.out.push('\n');
        self.col = 0;
        self.at_line_start = true;
    }

    /// Add `text` in the style of `role`. Unstyled text is written at once; styled text waits in a run.
    fn put(&mut self, role: Role, text: &str) {
        if self.theme.style(role) == anstyle::Style::new() {
            self.flush_run();
            self.out.push_str(text);
            return;
        }
        match &mut self.run {
            Some((r, t)) if *r == role => t.push_str(text),
            _ => {
                self.flush_run();
                self.run = Some((role, text.to_string()));
            }
        }
    }

    fn flush_run(&mut self) {
        if let Some((role, text)) = self.run.take() {
            self.out.push_str(&self.theme.paint(role, &text));
        }
    }

    /// A space between two words: part of the run when both words are in the same style.
    fn put_space(&mut self, next: Role) {
        match &mut self.run {
            Some((r, t)) if *r == next => t.push(' '),
            _ => {
                self.flush_run();
                self.out.push(' ');
            }
        }
    }

    fn flush_word(&mut self) {
        if self.word.is_empty() {
            return;
        }
        let word = std::mem::take(&mut self.word);
        let len = std::mem::take(&mut self.word_len);
        let space = usize::from(self.pending_space);
        if !self.at_line_start && self.col + space + len > self.width {
            self.break_line();
        } else if space == 1 {
            self.put_space(word[0].0);
            self.col += 1;
        }
        self.pending_space = false;
        self.start_line();
        if self.col + len <= self.width {
            for (role, text) in &word {
                self.put(*role, text);
            }
            self.col += len;
            return;
        }
        // longer than what is left of a whole line: cut it
        for (role, text) in &word {
            let mut piece = String::new();
            for ch in text.chars() {
                if self.col >= self.width {
                    self.put(*role, &piece);
                    piece.clear();
                    self.break_line();
                    self.start_line();
                }
                piece.push(ch);
                self.col += 1;
            }
            self.put(*role, &piece);
        }
    }
}

#[cfg(test)]
mod tests;
