//! The values the engine hands to a presenter, and what a presenter needs to know about the terminal.

use super::{detect_level, locale_is_utf8, ColorSetting, Level};
use crate::ports::Env;

/// What a message means, independent of how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Something to read, neither good nor bad.
    Info,
    /// Something worked.
    Success,
    /// Something was refused, skipped or is worth a second look.
    Warning,
    /// Something failed.
    Error,
    /// Background detail.
    Muted,
}

/// The symbol in front of a [`Notice`]. Which symbol is used on a terminal that cannot show Unicode is the
/// presenter's business.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Check,
    Cross,
    Arrow,
    Chevron,
}

impl Mark {
    pub fn glyph(self, unicode: bool) -> &'static str {
        match (self, unicode) {
            (Mark::Check, true) => "✔",
            (Mark::Check, false) => "+",
            (Mark::Cross, true) => "✘",
            (Mark::Cross, false) => "x",
            (Mark::Arrow, true) => "→",
            (Mark::Arrow, false) => "->",
            (Mark::Chevron, _) => ">",
        }
    }
}

/// Something the engine did while a request was running: a tool call that ran, was refused or was denied.
/// `text` and `detail` are already safe to show (escaped by whoever built them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub tone: Tone,
    pub mark: Mark,
    pub text: String,
    /// A short status after the text, shown in brackets (`exit 0, 0.1s`).
    pub meta: Option<String>,
    /// Further lines, shown under `text` (the first lines of a command's output).
    pub detail: Vec<String>,
    /// The answer text before this notice did not end its line, so the notice starts on a new one.
    pub gap: bool,
}

impl Notice {
    pub fn new(tone: Tone, mark: Mark, text: impl Into<String>) -> Self {
        Self {
            tone,
            mark,
            text: text.into(),
            meta: None,
            detail: Vec::new(),
            gap: false,
        }
    }

    pub fn with_meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn with_detail(mut self, detail: Vec<String>) -> Self {
        self.detail = detail;
        self
    }

    /// The notice as plain text, without the line break that [`Notice::gap`] asks for.
    pub fn plain(&self, unicode: bool) -> String {
        let mut s = format!("{} {}", self.mark.glyph(unicode), self.text);
        if let Some(meta) = &self.meta {
            s.push_str(&format!("   [{meta}]"));
        }
        for line in &self.detail {
            s.push_str("\n  ");
            s.push_str(line);
        }
        s
    }
}

/// A note shown after the answer. `text` is already safe to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remark {
    pub tone: Tone,
    /// What kind of note it is (`note`, `check`), shown before the text as `label: text`.
    pub label: Option<&'static str>,
    pub text: String,
    /// Lines listed under `text` (alternative commands).
    pub items: Vec<String>,
}

impl Remark {
    pub fn new(tone: Tone, text: impl Into<String>) -> Self {
        Self {
            tone,
            label: None,
            text: text.into(),
            items: Vec::new(),
        }
    }

    pub fn with_items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }

    pub fn labelled(tone: Tone, label: &'static str, text: impl Into<String>) -> Self {
        Self {
            tone,
            label: Some(label),
            text: text.into(),
            items: Vec::new(),
        }
    }

    /// The remark as plain text.
    pub fn plain(&self) -> String {
        let mut s = match self.label {
            Some(label) => format!("{label}: {}", self.text),
            None => self.text.clone(),
        };
        for item in &self.items {
            s.push_str("\n  ");
            s.push_str(item);
        }
        s
    }
}

/// What a presenter needs to know about the terminal it draws for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Columns available for text.
    pub width: usize,
    /// The terminal can show Unicode symbols (a UTF-8 locale).
    pub unicode: bool,
    /// How much styling the terminal can show.
    pub level: Level,
    /// Format the model's markdown (only at a level above plain).
    pub markdown: bool,
}

impl Caps {
    /// Text only, at `width` columns.
    pub const fn plain(width: usize, unicode: bool) -> Self {
        Self {
            width,
            unicode,
            level: Level::Plain,
            markdown: false,
        }
    }

    /// The caps for a stream, decided from the environment (see [`detect_level`]).
    pub fn detect(
        env: &dyn Env,
        is_terminal: bool,
        width: usize,
        setting: ColorSetting,
        markdown: bool,
    ) -> Self {
        let level = detect_level(env, is_terminal, setting);
        Self {
            width,
            unicode: locale_is_utf8(env),
            level,
            markdown: markdown && level > Level::Plain,
        }
    }
}
